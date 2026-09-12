//! Reading another process's memory on Windows.
//!
//! Everything here is about *how* to read, never *what* to read. No CS2
//! knowledge lives in this module, which is why a game update cannot touch
//! it.

use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::{CloseHandle, E_ACCESSDENIED, HANDLE};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, PROCESSENTRY32W,
    Process32FirstW, Process32NextW, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

/// Why attaching failed, separated from the raw error because the caller can
/// only act on some of these.
#[derive(Debug)]
pub enum AttachError {
    /// The executable is not running.
    NotRunning,
    /// Opening the process was refused — almost always missing elevation.
    AccessDenied,
    Other(windows::core::Error),
}

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRunning => write!(f, "the process is not running"),
            Self::AccessDenied => write!(f, "access denied — run as administrator"),
            Self::Other(error) => write!(f, "{error}"),
        }
    }
}

impl From<windows::core::Error> for AttachError {
    fn from(error: windows::core::Error) -> Self {
        Self::Other(error)
    }
}

/// One loaded module inside the target process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Module {
    pub base: usize,
    pub size: u32,
}

/// An open, read-only handle to another process. Closes itself on drop.
#[derive(Debug)]
pub struct Process {
    handle: HANDLE,
    pid: u32,
}

impl Process {
    /// Attach to the first process with this executable name.
    pub fn attach(exe_name: &str) -> Result<Self, AttachError> {
        let Some(pid) = find_pid(exe_name)? else {
            return Err(AttachError::NotRunning);
        };

        let handle =
            match unsafe { OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, false, pid) } {
                Ok(handle) => handle,
                Err(error) if error.code() == E_ACCESSDENIED => {
                    return Err(AttachError::AccessDenied);
                }
                Err(error) => return Err(error.into()),
            };

        Ok(Self { handle, pid })
    }

    pub const fn pid(&self) -> u32 {
        self.pid
    }

    /// Look up a loaded module by name. `None` means it is not loaded yet,
    /// which is normal while the game is still starting.
    pub fn module(&self, name: &str) -> windows::core::Result<Option<Module>> {
        // A 64-bit target ignores SNAPMODULE32 and a 32-bit one needs it, so
        // passing both works either way and costs nothing.
        let snapshot =
            unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, self.pid)? };
        let mut entry = MODULEENTRY32W {
            dwSize: size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };

        let mut found = None;
        if unsafe { Module32FirstW(snapshot, &mut entry) }.is_ok() {
            loop {
                if wide_to_string(&entry.szModule).eq_ignore_ascii_case(name) {
                    found = Some(Module {
                        base: entry.modBaseAddr as usize,
                        size: entry.modBaseSize,
                    });
                    break;
                }
                if unsafe { Module32NextW(snapshot, &mut entry) }.is_err() {
                    break;
                }
            }
        }

        unsafe { CloseHandle(snapshot)? };
        Ok(found)
    }

    /// Fill `out` from `address`. A short read is an error: a partially filled
    /// buffer is indistinguishable from real data once it is returned.
    pub fn read(&self, address: usize, out: &mut [u8]) -> windows::core::Result<()> {
        let mut read = 0usize;
        unsafe {
            ReadProcessMemory(
                self.handle,
                address as *const c_void,
                out.as_mut_ptr().cast(),
                out.len(),
                Some(&mut read),
            )?;
        }
        if read == out.len() {
            Ok(())
        } else {
            Err(windows::core::Error::from(E_ACCESSDENIED))
        }
    }

    pub fn read_f32(&self, address: usize) -> windows::core::Result<f32> {
        let mut bytes = [0u8; 4];
        self.read(address, &mut bytes)?;
        Ok(f32::from_le_bytes(bytes))
    }

    pub fn read_i32(&self, address: usize) -> windows::core::Result<i32> {
        let mut bytes = [0u8; 4];
        self.read(address, &mut bytes)?;
        Ok(i32::from_le_bytes(bytes))
    }

    /// Read a pointer. `None` for null, which callers almost always want to
    /// treat as an ordinary absence rather than dereference.
    pub fn read_pointer(&self, address: usize) -> windows::core::Result<Option<usize>> {
        let mut bytes = [0u8; 8];
        self.read(address, &mut bytes)?;
        let value = u64::from_le_bytes(bytes) as usize;
        Ok((value != 0).then_some(value))
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

fn find_pid(name: &str) -> windows::core::Result<Option<u32>> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)? };
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    let mut found = None;
    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            if wide_to_string(&entry.szExeFile).eq_ignore_ascii_case(name) {
                found = Some(entry.th32ProcessID);
                break;
            }
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }

    unsafe { CloseHandle(snapshot)? };
    Ok(found)
}

/// A UTF-16 fixed array as Windows fills it: NUL-terminated, rest is garbage.
fn wide_to_string(wide: &[u16]) -> String {
    let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}

#[cfg(test)]
mod tests {
    use super::wide_to_string;

    #[test]
    fn a_name_stops_at_the_terminator_and_ignores_the_garbage_after_it() {
        let mut buffer = [0u16; 8];
        for (slot, ch) in buffer.iter_mut().zip("cs2.exe".encode_utf16()) {
            *slot = ch;
        }
        buffer[7] = 0;
        assert_eq!(wide_to_string(&buffer), "cs2.exe");

        // Windows leaves whatever was in the buffer after the NUL.
        let mut dirty = buffer;
        dirty[4] = 0;
        dirty[5] = u16::from(b'X');
        assert_eq!(wide_to_string(&dirty), "cs2.");
    }

    #[test]
    fn a_full_buffer_with_no_terminator_still_reads_as_a_whole_name() {
        let full: Vec<u16> = "client.dll".encode_utf16().collect();
        assert_eq!(wide_to_string(&full), "client.dll");
    }
}
