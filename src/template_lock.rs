use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

const LOCK_FILE: &str = ".densevm-template-tree.lock";
const LOCK_SHARED: i32 = 1;
const LOCK_EXCLUSIVE: i32 = 2;
const LOCK_NONBLOCKING: i32 = 4;

#[link(name = "c")]
unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}

/// Holds the advisory lock used by publishers, clone startup, and collection.
pub struct TemplateTreeLock {
    _file: File,
}

impl TemplateTreeLock {
    pub fn acquire_exclusive(root: &Path, nonblocking: bool) -> Result<Self, String> {
        Self::acquire_with_mode(root, LOCK_EXCLUSIVE, nonblocking)
    }

    pub fn acquire_shared(root: &Path, nonblocking: bool) -> Result<Self, String> {
        Self::acquire_with_mode(root, LOCK_SHARED, nonblocking)
    }

    fn acquire_with_mode(root: &Path, mode: i32, nonblocking: bool) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|error| format!("resolve template-tree root {}: {error}", root.display()))?;
        if !root.is_dir() {
            return Err(format!(
                "template-tree root is not a directory: {}",
                root.display()
            ));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc_o_nofollow())
            .open(root.join(LOCK_FILE))
            .map_err(|error| format!("open template-tree lock: {error}"))?;
        let operation = mode | if nonblocking { LOCK_NONBLOCKING } else { 0 };
        // flock serializes with the template-tree Python publisher through the
        // shared .densevm-template-tree.lock inode.
        let result = unsafe { flock(file.as_raw_fd(), operation) };
        if result == -1 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                return Err(format!(
                    "template-tree operation is already active: {}",
                    root.display()
                ));
            }
            return Err(format!("lock template-tree root: {error}"));
        }
        Ok(Self { _file: file })
    }
}

#[cfg(target_os = "linux")]
fn libc_o_nofollow() -> i32 {
    0o400000
}

#[cfg(not(target_os = "linux"))]
fn libc_o_nofollow() -> i32 {
    0
}
