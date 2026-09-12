//! Reading the player's own mouse.
//!
//! Not where the cursor is — where the hand moved. Windows applies pointer
//! acceleration and screen edges to the cursor, and a game reads neither, so
//! the cursor says nothing about what the player did. Raw input reports the
//! device's own counts before any of that.
//!
//! This is what "the hand wins" is measured against. A tool that yields to the
//! player has to know what the player did, separately from what it did itself,
//! and the two are the same quantity arriving from different places.
//!
//! Registered with `INPUTSINK`, so the counts arrive while the game holds
//! focus. The overlay never takes focus — it could not, it is click-through —
//! so without that flag nothing would ever be delivered.
//!
//! **Nothing acts on these counts, and that is deliberate.** They are read and
//! shown so that the reading itself is known to work, and no decision consults
//! them. Wiring them in is its own step, taken when there is output for them
//! to yield to; until then a module that fed a decision nothing would read as
//! a missing connection rather than a choice.

use std::mem::size_of;

use windows::Win32::Devices::HumanInterfaceDevice::{
    HID_USAGE_GENERIC_MOUSE, HID_USAGE_PAGE_GENERIC,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, MOUSE_MOVE_ABSOLUTE, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RID_INPUT, RIDEV_INPUTSINK, RIM_TYPEMOUSE, RegisterRawInputDevices,
};
use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW, WM_INPUT};

/// The player's mouse, as the device reports it.
pub struct RawMouse {
    window: HWND,
    /// Counts since the last time they were taken.
    pending: [i64; 2],
    /// Packets that reported a position rather than a movement.
    ///
    /// Tablets, remote desktop and some virtual devices send absolute
    /// coordinates. There is no way to turn those into a movement without
    /// remembering where the pointer was, and mixing the two would produce
    /// enormous false movements, so they are counted and dropped instead.
    absolute: u64,
    /// Packets seen, so a count that stays at zero can be told from a mouse
    /// that is simply not moving.
    packets: u64,
}

impl RawMouse {
    /// Ask Windows to deliver this window the mouse's raw counts.
    pub fn listen(window: HWND) -> windows::core::Result<Self> {
        let device = RAWINPUTDEVICE {
            usUsagePage: HID_USAGE_PAGE_GENERIC,
            usUsage: HID_USAGE_GENERIC_MOUSE,
            // Deliver even while another window has focus — which is always,
            // since the game does and the overlay cannot.
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: window,
        };
        unsafe {
            RegisterRawInputDevices(&[device], size_of::<RAWINPUTDEVICE>() as u32)?;
        }
        Ok(Self {
            window,
            pending: [0; 2],
            absolute: 0,
            packets: 0,
        })
    }

    /// Drain whatever the device has sent since the last call, and say how
    /// many packets that was.
    ///
    /// Takes only its own messages out of the queue, so the overlay's pump can
    /// go on handling the rest without either having to know about the other.
    ///
    /// The count is returned rather than only accumulated because what this
    /// costs per frame is a number of packets times a cost each, and a total
    /// since startup cannot separate the two.
    pub fn poll(&mut self) -> u32 {
        let drained = self.packets;
        let mut message = MSG::default();
        // A bounded drain. An unbounded one would let a flood of packets hold
        // the frame open indefinitely, and a mouse cannot outrun this in the
        // time one frame takes.
        for _ in 0..256 {
            let waiting = unsafe {
                PeekMessageW(
                    &mut message,
                    Some(self.window),
                    WM_INPUT,
                    WM_INPUT,
                    PM_REMOVE,
                )
            };
            if !waiting.as_bool() {
                break;
            }
            self.absorb(HRAWINPUT(message.lParam.0 as *mut _));
        }
        // Packets we could not read are not counted by `absorb`, so this is
        // what arrived and was understood, not what the queue held.
        (self.packets - drained) as u32
    }

    fn absorb(&mut self, handle: HRAWINPUT) {
        let mut input = RAWINPUT::default();
        let mut size = size_of::<RAWINPUT>() as u32;
        let written = unsafe {
            GetRawInputData(
                handle,
                RID_INPUT,
                Some(std::ptr::from_mut(&mut input).cast()),
                &mut size,
                size_of::<RAWINPUTHEADER>() as u32,
            )
        };
        // The call returns the bytes written, or `u32::MAX` for a failure it
        // will not explain. Either way, a packet we could not read is a packet
        // we do not count.
        if written == 0 || written == u32::MAX || input.header.dwType != RIM_TYPEMOUSE.0 {
            return;
        }

        self.packets += 1;
        let mouse = unsafe { input.data.mouse };
        if mouse.usFlags.0 & MOUSE_MOVE_ABSOLUTE.0 != 0 {
            self.absolute += 1;
            return;
        }
        self.pending[0] += i64::from(mouse.lLastX);
        self.pending[1] += i64::from(mouse.lLastY);
    }

    /// The counts since this was last called, and reset.
    pub fn take(&mut self) -> [i64; 2] {
        std::mem::take(&mut self.pending)
    }

    pub const fn packets(&self) -> u64 {
        self.packets
    }

    /// Packets dropped for reporting a position instead of a movement. A
    /// number that climbs means the device is not one this can read.
    pub const fn absolute(&self) -> u64 {
        self.absolute
    }
}
