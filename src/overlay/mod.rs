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

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, ClientToScreen, CreateCompatibleBitmap,
    CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush, DEFAULT_CHARSET, DeleteDC,
    DeleteObject, FF_DONTCARE, FW_SEMIBOLD, FillRect, GetDC, HDC, HFONT, HGDIOBJ, LineTo, MoveToEx,
    OUT_DEFAULT_PRECIS, PS_SOLID, ReleaseDC, SRCCOPY, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT as BK_TRANSPARENT, TextOutW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowW, GetClientRect, HWND_TOPMOST,
    IsWindow, LWA_COLORKEY, MSG, PM_REMOVE, PeekMessageW, RegisterClassExW, SW_SHOW,
    SWP_NOACTIVATE, SetLayeredWindowAttributes, SetWindowPos, ShowWindow, TranslateMessage,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

/// The colour treated as see-through. Nothing may be drawn in it.
///
/// Not pure black: black is too useful to give up, and a near-black is
/// indistinguishable to the eye anywhere it does leak through.
const KEY: COLORREF = COLORREF(0x01_01_01);

/// A colour in the order GDI wants it, which is not the order it is written.
pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((b as u32) << 16 | (g as u32) << 8 | r as u32)
}

/// Where the game's client area sits on screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub struct Overlay {
    window: HWND,
    target: HWND,
    bounds: Bounds,
    font: HFONT,
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
        }))
    }

    pub const fn bounds(&self) -> Bounds {
        self.bounds
    }

    /// Whether the game window is still there.
    pub fn target_is_alive(&self) -> bool {
        unsafe { IsWindow(Some(self.target)).as_bool() }
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
        self.bounds = bounds;
        unsafe {
            let _ = SetWindowPos(
                self.window,
                Some(HWND_TOPMOST),
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                SWP_NOACTIVATE,
            );
        }
    }

    /// Handle any window messages waiting for us.
    ///
    /// A window that never reads its queue is reported as hung by Windows,
    /// which eventually draws a ghost copy over it.
    pub fn pump(&self) {
        let mut message = MSG::default();
        unsafe {
            while PeekMessageW(&mut message, Some(self.window), 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    /// Draw one frame. Everything `paint` draws lands on screen together.
    pub fn frame(&self, paint: impl FnOnce(&mut Canvas)) {
        unsafe {
            let window_dc = GetDC(Some(self.window));
            if window_dc.is_invalid() {
                return;
            }
            let buffer_dc = CreateCompatibleDC(Some(window_dc));
            let bitmap = CreateCompatibleBitmap(window_dc, self.bounds.width, self.bounds.height);
            let previous = SelectObject(buffer_dc, HGDIOBJ(bitmap.0));

            // Start from the key colour: every pixel not drawn on is a hole.
            let key_brush = CreateSolidBrush(KEY);
            let whole = RECT {
                left: 0,
                top: 0,
                right: self.bounds.width,
                bottom: self.bounds.height,
            };
            FillRect(buffer_dc, &whole, key_brush);
            let _ = DeleteObject(HGDIOBJ(key_brush.0));

            SelectObject(buffer_dc, HGDIOBJ(self.font.0));
            SetBkMode(buffer_dc, BK_TRANSPARENT);

            let mut canvas = Canvas { dc: buffer_dc };
            paint(&mut canvas);

            let _ = BitBlt(
                window_dc,
                0,
                0,
                self.bounds.width,
                self.bounds.height,
                Some(buffer_dc),
                0,
                0,
                SRCCOPY,
            );

            SelectObject(buffer_dc, previous);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(buffer_dc);
            ReleaseDC(Some(self.window), window_dc);
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.font.0));
        }
    }
}

/// What a frame can draw on. Coordinates are relative to the game's client
/// area, so `(0, 0)` is its top-left corner rather than the screen's.
pub struct Canvas {
    dc: HDC,
}

impl Canvas {
    pub fn line(&mut self, from: (i32, i32), to: (i32, i32), colour: COLORREF, width: i32) {
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
