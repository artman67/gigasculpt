//! `gs-win32`: the Win32 calls the standard library has no safe form of, one public function
//! each, and nothing else. It is the workspace's only exception to `unsafe_code = "forbid"`
//! (decision 0065): every other crate takes the workspace's lints, while this one has its own,
//! which deny unsafe code outside the functions here and make every unsafe block say why it is
//! sound. On Linux and macOS it is empty.
//!
//! Delete it when the pinned toolchain (`rust-toolchain.toml`) has
//! `std::os::windows::process::CommandExt::spawn_with_attributes` and `ProcThreadAttributeList`
//! stable (Rust issue 114854): `gs` then hands the engine a list of exactly the handles it is
//! meant to get, and needs nothing here.

#[cfg(windows)]
use std::io;
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, BorrowedHandle};

/// Keeps `handle` from the programs this process starts, by clearing its inherit flag
/// (`SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0)`). A handle the standard library hands
/// a child on purpose, as `Stdio::inherit()` does, still reaches it: std passes an inheritable
/// copy made for that one start. Fails for a null handle, which a process without a console has.
#[cfg(windows)]
#[expect(
    unsafe_code,
    reason = "a Win32 call std has no safe form of (decision 0065)"
)]
pub fn keep_from_children(handle: BorrowedHandle<'_>) -> io::Result<()> {
    use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
    // SAFETY: `handle` is borrowed, so it stays open for the call (a null one is refused with an
    // error); SetHandleInformation reads only the handle's value and changes only its flags.
    let done = unsafe { SetHandleInformation(handle.as_raw_handle(), HANDLE_FLAG_INHERIT, 0) };
    if done == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
