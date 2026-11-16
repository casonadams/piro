//! SIGINT, SIGTERM and SIGHUP end the run loop, so the terminal is restored, instead of killing the process.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn requested() -> bool {
    REQUESTED.load(Ordering::Relaxed)
}

extern "C" fn request(_: libc::c_int) {
    REQUESTED.store(true, Ordering::Relaxed);
}

/// Installs the handlers and puts back the previous ones on drop.
pub struct Guard(Vec<(libc::c_int, libc::sigaction)>);

impl Guard {
    pub fn install() -> io::Result<Self> {
        let mut guard = Self(Vec::new());
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            guard.0.push((signal, install_one(signal)?));
        }
        Ok(guard)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        for (signal, previous) in &self.0 {
            // SAFETY: restores an action this guard saved from sigaction when it was installed.
            unsafe {
                libc::sigaction(*signal, previous, std::ptr::null_mut());
            }
        }
    }
}

/// Installs the flag-setting handler for `signal`, returning the action it replaced.
fn install_one(signal: libc::c_int) -> io::Result<libc::sigaction> {
    // SAFETY: zeroed sigaction structs are valid starting values; the handler only stores an atomic flag.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        let mut previous: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = request as *const () as libc::sighandler_t;
        libc::sigemptyset(&mut action.sa_mask);
        if libc::sigaction(signal, &action, &mut previous) != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(previous)
    }
}
