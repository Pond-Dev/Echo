//! Step 1: attach to CS2 and prove we can read its memory.
//!
//! Finds `cs2.exe`, locates `client.dll` inside it, and reads the first two
//! bytes at the module base. A PE image starts with `MZ`, so seeing it back
//! proves the whole chain works: process lookup, handle, module enumeration,
//! and a cross-process read.
//!
//! No game offsets are involved on purpose — those are a separate problem.

use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::{CloseHandle, E_ACCESSDENIED, HANDLE};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, PROCESSENTRY32W,
    Process32FirstW, Process32NextW, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

const GAME_EXE: &str = "cs2.exe";
const GAME_MODULE: &str = "client.dll";

/// A UTF-16 fixed array as Windows fills it: NUL-terminated, rest is garbage.
fn wide_to_string(wide: &[u16]) -> String {
    let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}

fn find_process(name: &str) -> windows::core::Result<Option<u32>> {
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

/// Base address and size of one loaded module, by name.
fn find_module(pid: u32, name: &str) -> windows::core::Result<Option<(usize, u32)>> {
    // A 64-bit target still wants both flags: SNAPMODULE32 is ignored for a
    // 64-bit process and required for a 32-bit one, so passing both works
    // either way and costs nothing.
    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid)? };
    let mut entry = MODULEENTRY32W {
        dwSize: size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };

    let mut found = None;
    if unsafe { Module32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            if wide_to_string(&entry.szModule).eq_ignore_ascii_case(name) {
                found = Some((entry.modBaseAddr as usize, entry.modBaseSize));
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

fn read_bytes(process: HANDLE, address: usize, out: &mut [u8]) -> windows::core::Result<usize> {
    let mut read = 0usize;
    unsafe {
        ReadProcessMemory(
            process,
            address as *const c_void,
            out.as_mut_ptr().cast(),
            out.len(),
            Some(&mut read),
        )?;
    }
    Ok(read)
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

fn main() {
    let result = run();
    if let Err(error) = &result {
        println!("Failed: {error}");
    }
    wait_before_closing();
}

fn run() -> windows::core::Result<()> {
    let Some(pid) = find_process(GAME_EXE)? else {
        println!("{GAME_EXE} is not running.");
        return Ok(());
    };
    println!("{GAME_EXE}  pid={pid}");

    // Reading another process needs elevation (or SeDebugPrivilege). Say so
    // instead of surfacing a bare HRESULT: this is the first wall anyone hits.
    let process =
        match unsafe { OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, false, pid) } {
            Ok(handle) => handle,
            Err(error) if error.code() == E_ACCESSDENIED => {
                println!("Cannot open the process: access denied.");
                println!("Run this from an elevated terminal (Administrator).");
                return Ok(());
            }
            Err(error) => return Err(error),
        };

    let result = (|| -> windows::core::Result<()> {
        let Some((base, size)) = find_module(pid, GAME_MODULE)? else {
            println!("{GAME_MODULE} is not loaded yet.");
            return Ok(());
        };
        println!("{GAME_MODULE}  base=0x{base:X}  size={size} bytes");

        let mut header = [0u8; 2];
        let read = read_bytes(process, base, &mut header)?;
        println!("read {read} bytes at base: {header:02X?}  ({:?})", {
            let printable: String = header.iter().map(|&b| b as char).collect();
            printable
        });

        if &header == b"MZ" {
            println!("\nPE signature matches. Attach and cross-process read both work.");
        } else {
            println!("\nExpected 'MZ' at the module base. Something is wrong.");
        }
        Ok(())
    })();

    unsafe { CloseHandle(process)? };
    result
}

/// Double-clicking a console binary closes the window the moment it returns,
/// so the output is unreadable. Hold it open until a key is pressed.
fn wait_before_closing() {
    println!("\nPress Enter to close.");
    let _ = std::io::stdin().read_line(&mut String::new());
}
