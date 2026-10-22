//! Convert SIGINT/SIGTERM into ordinary errors so owned VMs and KSM guards drop.
use std::sync::atomic::{AtomicBool, Ordering};
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn handle(_: std::os::raw::c_int) {
    INTERRUPTED.store(true, Ordering::Relaxed);
}

pub fn install() -> Result<(), String> {
    unsafe extern "C" {
        fn signal(
            number: std::os::raw::c_int,
            handler: extern "C" fn(std::os::raw::c_int),
        ) -> usize;
    }
    for number in [2, 15] {
        if unsafe { signal(number, handle) } == usize::MAX {
            return Err(format!(
                "install signal handler: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}

pub fn check() -> Result<(), String> {
    if INTERRUPTED.load(Ordering::Relaxed) {
        Err("interrupted; cleaning up owned QEMU processes".into())
    } else {
        Ok(())
    }
}
