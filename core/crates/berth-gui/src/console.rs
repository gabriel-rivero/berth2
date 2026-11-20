//! Windows console attachment for a GUI-subsystem binary.
//!
//! The window builds with `windows_subsystem = "windows"` so double-clicking it
//! does not open a console. Started from a terminal instead, it attaches to that
//! terminal so `--help`, `--version`, the launch banner and errors still print
//! where the user typed the command. Nothing here runs on other platforms.

/// Point stdout, stderr and stdin at the parent console when there is one.
///
/// Called before the first write in `main`: the standard library resolves the
/// process handles lazily, on first use.
pub fn attach_parent_console() {
    #[cfg(windows)]
    windows::attach();
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::ptr;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            attributes: *const c_void,
            disposition: u32,
            flags: u32,
            template: *mut c_void,
        ) -> *mut c_void;
        fn GetFileType(handle: *mut c_void) -> u32;
        fn GetStdHandle(which: u32) -> *mut c_void;
        fn SetStdHandle(which: u32, handle: *mut c_void) -> i32;
    }

    /// `ATTACH_PARENT_PROCESS`: the console of whoever started us.
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    const OPEN_EXISTING: u32 = 3;
    const STD_INPUT_HANDLE: u32 = -10i32 as u32;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const FILE_TYPE_UNKNOWN: u32 = 0;
    const FILE_TYPE_DISK: u32 = 1;
    const FILE_TYPE_PIPE: u32 = 3;
    const INVALID_HANDLE_VALUE: *mut c_void = usize::MAX as *mut c_void;

    pub fn attach() {
        // SAFETY: plain Win32 calls with owned buffers that outlive each call;
        // failure only means there was no console to attach to.
        unsafe {
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                return;
            }
            for (which, name) in [
                (STD_OUTPUT_HANDLE, "CONOUT$"),
                (STD_ERROR_HANDLE, "CONOUT$"),
                (STD_INPUT_HANDLE, "CONIN$"),
            ] {
                let existing = GetStdHandle(which);
                let kind = if existing.is_null() || existing == INVALID_HANDLE_VALUE {
                    FILE_TYPE_UNKNOWN
                } else {
                    GetFileType(existing)
                };
                if kind == FILE_TYPE_DISK || kind == FILE_TYPE_PIPE {
                    // Redirected to a file or a pipe (`berth-gui --help | less`);
                    // replacing that would lose the output.
                    continue;
                }
                let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
                let handle = CreateFileW(
                    wide.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    ptr::null(),
                    OPEN_EXISTING,
                    0,
                    ptr::null_mut(),
                );
                if handle != INVALID_HANDLE_VALUE {
                    SetStdHandle(which, handle);
                }
            }
        }
    }
}
