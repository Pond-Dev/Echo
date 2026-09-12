//! A window that draws on top of the game.
//!
//! The overlay is a borderless, always-on-top window sized to the game's
//! client area. Three flags do the real work:
//!
//! * `LAYERED` plus a colour key, so one colour is treated as see-through and
//!   everything we do not draw on shows the game underneath.
//! * `TRANSPARENT`, so clicks pass through to the game. Without it the overlay
//!   would swallow every mouse button the moment it covered the screen.
//! * `TOOLWINDOW`, to stay out of the alt-tab list.
//!
//! Frames are drawn into an off-screen bitmap and copied over in one go.
//! Drawing straight to the window means the screen is briefly the key colour
//! between clearing and drawing, which reads as a hard flicker at any frame
//! rate.

pub mod dirty;

use std::time::{Duration, Instant};

use dirty::{Dirty, Rect};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, ClientToScreen, CreateCompatibleBitmap,
    CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush, DEFAULT_CHARSET, DeleteDC,
    DeleteObject, FF_DONTCARE, FW_SEMIBOLD, FillRect, GetDC, GetTextExtentPoint32W, HBITMAP,
    HBRUSH, HDC, HFONT, HGDIOBJ, LineTo, MoveToEx, OUT_DEFAULT_PRECIS, PS_SOLID, ReleaseDC,
    SRCCOPY, SelectObject, SetBkMode, SetTextColor, TRANSPARENT as BK_TRANSPARENT, TextOutW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW, GetClientRect,
    GetForegroundWindow, HWND_TOPMOST, IsWindow, LWA_COLORKEY, MSG, PM_REMOVE, PeekMessageW,
    RegisterClassExW, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetLayeredWindowAttributes,
    SetWindowPos, ShowWindow, TranslateMessage, WM_INPUT, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

/// The colour treated as see-through. Nothing may be drawn in it.
///
/// Not pure black: black is too useful to give up, and a near-black is
/// indistinguishable to the eye anywhere it does leak through.
const KEY: COLORREF = COLORREF(0x01_01_01);

/// How often the topmost claim is repeated when nothing has moved.
///
/// Another always-on-top window can be raised above us at any time, so the
/// claim cannot be made once — but it does not have to be made every frame.
const TOPMOST_INTERVAL: Duration = Duration::from_millis(250);

/// A colour in the order GDI wants it, which is not the order it is written.
pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((b as u32) << 16 | (g as u32) << 8 | r as u32)
}

/// Where a frame's drawing time went.
///
/// The overlay was measured at ninety per cent of a frame and fixed by
/// replacing one line; what is left is still a single number, which says
/// nothing about which part of it to attack next. Splitting it is cheaper than
/// guessing — the last guess about where the time went was wrong by an order
/// of magnitude.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameCost {
    /// Getting and releasing the window's drawing context.
    pub surface: Duration,
    /// Reusing the off-screen surface, or building it when the size changed.
    pub buffer: Duration,
    /// Filling it with the key colour.
    pub clear: Duration,
    /// Whatever the caller drew: pens, lines, text.
    pub paint: Duration,
    /// Copying the finished surface to the window.
    pub blit: Duration,
    /// How many separate regions that took. One means the whole screen.
    pub regions: usize,
}

impl FrameCost {
    pub fn total(self) -> Duration {
        self.surface + self.buffer + self.clear + self.paint + self.blit
    }

    /// Worst first, with each share of the drawing.
    pub fn breakdown(self) -> Vec<String> {
        let total = self.total().as_secs_f64().max(1e-9);
        let regions = self.regions;
        let mut rows = [
            ("  clear", self.clear),
            ("  paint", self.paint),
            ("  blit", self.blit),
            ("  buffer", self.buffer),
            ("  surface", self.surface),
        ];
        rows.sort_by_key(|(_, took)| std::cmp::Reverse(*took));
        std::iter::once(format!("    {regions} region(s) cleared and copied"))
            .chain(rows.iter().map(|(name, took)| {
                let ms = took.as_secs_f64() * 1000.0;
                format!(
                    "  {name:<14} {ms:>7.3} ms   {:>4.1}%",
                    ms / (total * 1000.0) * 100.0
                )
            }))
            .collect()
    }
}

/// The Win32 rectangle for one of ours.
const fn as_rect(region: Rect) -> RECT {
    RECT {
        left: region.left,
        top: region.top,
        right: region.right,
        bottom: region.bottom,
    }
}

fn timed<T>(slot: &mut Duration, work: impl FnOnce() -> T) -> T {
    let started = std::time::Instant::now();
    let value = work();
    *slot = started.elapsed();
    value
}

/// Where the game's client area sits on screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The off-screen surface frames are drawn into.
///
/// Kept between frames. At 2048x1152 the bitmap is over nine megabytes, and
/// building one every frame was measured at ninety per cent of the frame —
/// more than everything else in the program put together, reading the game
/// included.
struct Buffer {
    /// Whether GDI actually gave us what we asked for.
    usable: bool,
    dc: HDC,
    bitmap: HBITMAP,
    /// What it was built for. A different size means it has to be rebuilt.
    size: (i32, i32),
    /// The object the bitmap displaced, needed to put the DC back before the
    /// bitmap can be freed.
    previous: HGDIOBJ,
    /// The key-coloured brush that clears it, also kept rather than rebuilt.
    key: HBRUSH,
}

impl Buffer {
    unsafe fn new(window_dc: HDC, size: (i32, i32)) -> Self {
        unsafe {
            let dc = CreateCompatibleDC(Some(window_dc));
            let bitmap = CreateCompatibleBitmap(window_dc, size.0, size.1);
            let previous = SelectObject(dc, HGDIOBJ(bitmap.0));
            // A failed allocation leaves the DC on its default monochrome
            // bitmap, which is not the key colour — so every frame would copy
            // an opaque rectangle over the game with nothing said about it.
            let usable = !dc.is_invalid() && !bitmap.is_invalid();
            Self {
                usable,
                dc,
                bitmap,
                size,
                previous,
                key: CreateSolidBrush(KEY),
            }
        }
    }

    unsafe fn release(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteObject(HGDIOBJ(self.key.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

pub struct Overlay {
    window: HWND,
    target: HWND,
    bounds: Bounds,
    font: HFONT,
    buffer: Option<Buffer>,
    /// What this frame marked, and what the one before it did. The previous
    /// frame's marks have to be cleared and copied too, or its drawing stays
    /// on screen after whatever made it has moved.
    dirty: Dirty,
    previous_dirty: Dirty,
    /// When the topmost claim was last made, or `None` if never.
    topmost_asserted: Option<Instant>,
}

impl Overlay {
    /// Create an overlay covering the window with this title.
    ///
    /// `None` means the game's window is not there — it has not opened yet, or
    /// it has closed. Either is ordinary.
    pub fn over(title: PCWSTR) -> windows::core::Result<Option<Self>> {
        let Some(target) = find_window(title) else {
            return Ok(None);
        };
        let Some(bounds) = client_bounds(target) else {
            return Ok(None);
        };

        let class = w!("echo_overlay");
        let window = unsafe {
            let wnd_class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(window_proc),
                lpszClassName: class,
                ..Default::default()
            };
            // Re-registering the same class fails harmlessly; only the first
            // call in a process needs to succeed.
            RegisterClassExW(&wnd_class);

            let window = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
                class,
                w!("echo"),
                WS_POPUP | WS_VISIBLE,
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                None,
                None,
                None,
                None,
            )?;

            SetLayeredWindowAttributes(window, KEY, 0, LWA_COLORKEY)?;
            let _ = ShowWindow(window, SW_SHOW);
            window
        };

        let font = unsafe {
            CreateFontW(
                16,
                0,
                0,
                0,
                FW_SEMIBOLD.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                FF_DONTCARE.0.into(),
                w!("Consolas"),
            )
        };

        Ok(Some(Self {
            window,
            target,
            bounds,
            font,
            buffer: None,
            dirty: Dirty::default(),
            previous_dirty: Dirty::default(),
            // `None` so the first frame makes the claim rather than waiting.
            // Subtracting the interval from `now` would be the obvious way and
            // panics: on Windows the clock starts at boot, so launching within
            // a quarter second of one aborts before the window is returned.
            topmost_asserted: None,
        }))
    }

    /// The overlay's own window, for anything that has to be told where to
    /// deliver messages.
    pub const fn window(&self) -> HWND {
        self.window
    }

    pub const fn bounds(&self) -> Bounds {
        self.bounds
    }

    /// Whether the game window is still there.
    pub fn target_is_alive(&self) -> bool {
        unsafe { IsWindow(Some(self.target)).as_bool() }
    }

    /// Whether the game is the window the player is actually in.
    ///
    /// Asked because sent movement goes wherever the focus is. With the game
    /// behind a browser, the same counts that would have turned the view drag
    /// the pointer across whatever is in front instead — so this is what
    /// stands between a tool that aims and a tool that grabs the desktop.
    pub fn target_has_focus(&self) -> bool {
        unsafe { GetForegroundWindow() == self.target }
    }

    /// Keep the overlay over the game, and on top of it.
    ///
    /// The game can be moved, resized or alt-tabbed away from at any moment,
    /// and another always-on-top window can be raised above us, so both are
    /// re-asserted every frame rather than set once.
    pub fn follow_target(&mut self) {
        let Some(bounds) = client_bounds(self.target) else {
            return;
        };

        let moved = bounds != self.bounds;
        let reassert = self
            .topmost_asserted
            .is_none_or(|at| at.elapsed() >= TOPMOST_INTERVAL);
        if !moved && !reassert {
            return;
        }
        self.bounds = bounds;
        // A move asserts topmost too, so it restarts the timer as much as a
        // deliberate re-assert does. Leaving it stale would fire a second,
        // redundant call right behind every move — on a window being dragged,
        // exactly the per-frame call this is here to avoid.
        self.topmost_asserted = Some(Instant::now());

        // Moving a window is a conversation with the window manager, and it
        // occasionally blocks: measured at sixty microseconds most frames and
        // over a millisecond in the worst, more than half of that frame. It
        // also sends messages back that the loop then pays to read, so calling
        // it every frame was work the overlay was making for itself.
        //
        // Skipping the move where nothing moved leaves only the topmost claim,
        // which has to be repeated because another always-on-top window can be
        // raised above us at any time — but a few times a second is enough for
        // that, and it is cheaper without the move.
        let flags = if moved {
            SWP_NOACTIVATE
        } else {
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE
        };
        unsafe {
            let _ = SetWindowPos(
                self.window,
                Some(HWND_TOPMOST),
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                flags,
            );
        }
    }

    /// Handle any window messages waiting for us.
    ///
    /// A window that never reads its queue is reported as hung by Windows,
    /// which eventually draws a ghost copy over it.
    ///
    /// Everything except the mouse's own. Those belong to whoever reads the
    /// player's hand, and taking them here meant every packet that arrived
    /// after that read and before this one was removed and thrown away —
    /// a share of the player's movement, every frame, silently missing from
    /// the figure that decides when the assist lets go.
    pub fn pump(&self) {
        let mut message = MSG::default();
        unsafe {
            for (first, last) in [(0, WM_INPUT - 1), (WM_INPUT + 1, u32::MAX)] {
                while PeekMessageW(&mut message, Some(self.window), first, last, PM_REMOVE)
                    .as_bool()
                {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
    }
    /// Draw one frame. Everything `paint` draws lands on screen together.
    pub fn frame(&mut self, paint: impl FnOnce(&mut Canvas<'_>)) -> FrameCost {
        let size = (self.bounds.width, self.bounds.height);
        let mut cost = FrameCost::default();
        unsafe {
            let window_dc = timed(&mut cost.surface, || GetDC(Some(self.window)));
            if window_dc.is_invalid() {
                return cost;
            }

            let mut rebuilt = false;
            timed(&mut cost.buffer, || {
                // Rebuilt only when the game is resized, which is rare enough
                // that the cost never shows up.
                if self.buffer.as_ref().is_none_or(|b| b.size != size) {
                    if let Some(old) = self.buffer.as_mut() {
                        old.release();
                    }
                    self.buffer = Some(Buffer::new(window_dc, size));
                    rebuilt = true;
                }
            });
            if rebuilt {
                // Nothing on a new bitmap is the key colour yet, and neither
                // is the window it will be copied to. Until every pixel has
                // been painted once, the parts left alone are opaque and hide
                // the game rather than showing it.
                self.previous_dirty.mark_everything();
            }
            let buffer = self.buffer.as_ref().expect("just built");
            if !buffer.usable {
                ReleaseDC(Some(self.window), window_dc);
                return cost;
            }
            // Where this frame draws is not known until it has drawn, so the
            // regions are settled after painting. Clearing can go first, and
            // only over what the previous frame left behind.
            let stale = self
                .previous_dirty
                .combined(&Dirty::default(), size.0, size.1);
            timed(&mut cost.clear, || {
                // Every pixel not drawn on has to be the key colour, or it
                // would cover the game rather than show it.
                for region in &stale {
                    FillRect(buffer.dc, &as_rect(*region), buffer.key);
                }
            });

            SelectObject(buffer.dc, HGDIOBJ(self.font.0));
            SetBkMode(buffer.dc, BK_TRANSPARENT);

            self.dirty.clear(size.0, size.1);
            timed(&mut cost.paint, || {
                let mut canvas = Canvas {
                    dc: buffer.dc,
                    dirty: &mut self.dirty,
                };
                paint(&mut canvas);
            });

            let regions = self.dirty.combined(&self.previous_dirty, size.0, size.1);
            cost.regions = regions.len();
            timed(&mut cost.blit, || {
                for region in &regions {
                    let _ = BitBlt(
                        window_dc,
                        region.left,
                        region.top,
                        region.right - region.left,
                        region.bottom - region.top,
                        Some(buffer.dc),
                        region.left,
                        region.top,
                        SRCCOPY,
                    );
                }
            });
            std::mem::swap(&mut self.previous_dirty, &mut self.dirty);

            let mut release = Duration::ZERO;
            timed(&mut release, || {
                ReleaseDC(Some(self.window), window_dc);
            });
            cost.surface += release;
        }
        cost
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            if let Some(buffer) = self.buffer.as_mut() {
                buffer.release();
            }
            // Without this the window outlives the struct: a topmost,
            // click-through window still showing the last frame's boxes over
            // the desktop, with nothing left running to pump its messages.
            let _ = DestroyWindow(self.window);
            let _ = DeleteObject(HGDIOBJ(self.font.0));
        }
    }
}

/// What a frame can draw on. Coordinates are relative to the game's client
/// area, so `(0, 0)` is its top-left corner rather than the screen's.
pub struct Canvas<'a> {
    dc: HDC,
    /// What this frame has marked, so only that has to be cleared and copied.
    dirty: &'a mut Dirty,
}

impl Canvas<'_> {
    pub fn line(&mut self, from: (i32, i32), to: (i32, i32), colour: COLORREF, width: i32) {
        // A pen paints half its width either side of the path, and a rounded
        // end cap reaches a little past each endpoint. One extra pixel covers
        // both without having to reason about cap shapes.
        self.dirty.touch(Rect::around(from, to, width / 2 + 1));
        unsafe {
            let pen = CreatePen(PS_SOLID, width, colour);
            let previous = SelectObject(self.dc, HGDIOBJ(pen.0));
            let mut ignored = POINT::default();
            let _ = MoveToEx(self.dc, from.0, from.1, Some(&mut ignored));
            let _ = LineTo(self.dc, to.0, to.1);
            SelectObject(self.dc, previous);
            let _ = DeleteObject(HGDIOBJ(pen.0));
        }
    }

    pub fn rect(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        colour: COLORREF,
        thickness: i32,
    ) {
        let (right, bottom) = (x + width, y + height);
        self.line((x, y), (right, y), colour, thickness);
        self.line((right, y), (right, bottom), colour, thickness);
        self.line((right, bottom), (x, bottom), colour, thickness);
        self.line((x, bottom), (x, y), colour, thickness);
    }

    pub fn text(&mut self, x: i32, y: i32, text: &str, colour: COLORREF) {
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            SetTextColor(self.dc, colour);
            let _ = TextOutW(self.dc, x, y, &wide);
            // Ask for the extent rather than guessing from the character
            // count: a guess that is short leaves the tail of the line on
            // screen after it changes, which is exactly the smear this is
            // meant to prevent.
            let mut size = SIZE::default();
            let width = if GetTextExtentPoint32W(self.dc, &wide, &mut size).as_bool() {
                size.cx
            } else {
                wide.len() as i32 * 16
            };
            self.dirty
                .touch(Rect::new(x, y, x + width + 2, y + size.cy.max(20) + 2));
        }
    }
}

fn find_window(title: PCWSTR) -> Option<HWND> {
    let window = unsafe { FindWindowW(None, title) }.ok()?;
    (!window.is_invalid()).then_some(window)
}

/// The target's drawable area, in screen coordinates.
///
/// The client area rather than the whole window: the title bar and borders are
/// not part of the game's picture, and in fullscreen there are none anyway.
fn client_bounds(window: HWND) -> Option<Bounds> {
    let mut rect = RECT::default();
    unsafe { GetClientRect(window, &mut rect) }.ok()?;

    let mut origin = POINT { x: 0, y: 0 };
    unsafe { ClientToScreen(window, &mut origin) }.ok().ok()?;

    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    (width > 0 && height > 0).then_some(Bounds {
        x: origin.x,
        y: origin.y,
        width,
        height,
    })
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}
