//! Tiny Linux guest used only to prove QEMU state restore and COW isolation.
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::raw::{c_char, c_int, c_long, c_ulong, c_void};
use std::thread;
use std::time::Duration;

unsafe extern "C" {
    fn mount(source: *const c_char, target: *const c_char, kind: *const c_char, flags: c_ulong, data: *const c_void) -> c_int;
    fn syscall(number: c_long, ...) -> c_long;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn sethostname(name: *const c_char, len: usize) -> c_int;
}

fn load_module(name: &str) {
    let file = File::open(format!("/modules/{name}.ko.zst")).expect("open guest network module");
    let params = CString::new("").unwrap();
    let code = unsafe { syscall(313, file.as_raw_fd(), params.as_ptr(), 4 as c_int) };
    assert_eq!(code, 0, "load {name}: {}", std::io::Error::last_os_error());
}

fn mount_fs(source: &str, target: &str, kind: &str) {
    fs::create_dir_all(target).expect("create mount point");
    let source = CString::new(source).unwrap();
    let target = CString::new(target).unwrap();
    let kind = CString::new(kind).unwrap();
    let code = unsafe { mount(source.as_ptr(), target.as_ptr(), kind.as_ptr(), 0, std::ptr::null()) };
    assert_eq!(code, 0, "mount failed: {}", std::io::Error::last_os_error());
}

fn disk_byte() -> std::io::Result<u8> {
    let mut file = File::open("/dev/vda")?;
    let mut byte = [0u8; 1];
    file.read_exact(&mut byte)?;
    Ok(byte[0])
}

fn write_disk(value: u8) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).open("/dev/vda")?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&[value; 4096])?;
    file.sync_all()
}

fn fill_disk(value: u8) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).open("/dev/vda")?;
    let block = [value; 1024 * 1024];
    for _ in 0..64 {
        file.write_all(&block)?;
    }
    file.sync_all()
}

fn reboot_guest() -> ! {
    let result = unsafe { syscall(169, 0xfee1dead_u32, 672274793_u32, 0x01234567_u32, 0_u32) };
    panic!("reboot syscall failed: {result}");
}

fn valid_identity(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn set_identity(value: &str) -> std::io::Result<()> {
    if !valid_identity(value) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "identity must be 32 hexadecimal characters",
        ));
    }
    fs::create_dir_all("/etc")?;
    fs::write("/etc/machine-id", format!("{value}\n"))?;
    let hostname = format!("densevm-{}", &value[..12]);
    fs::write("/etc/hostname", format!("{hostname}\n"))?;
    if unsafe { sethostname(hostname.as_ptr().cast(), hostname.len()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn reseed(value: &str) -> std::io::Result<()> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "expected 32 bytes of host entropy"));
    }
    // Linux UAPI linux/random.h: rand_pool_info and RNDADDENTROPY,
    // followed by RNDRESEEDCRNG to force a new CRNG key before networking.
    #[repr(C)]
    struct Pool { entropy_count: c_int, size: c_int, bytes: [u8; 32] }
    let mut pool = Pool { entropy_count: 256, size: 32, bytes: [0; 32] };
    for (i, byte) in pool.bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i*2..i*2+2], 16).unwrap();
    }
    let random = OpenOptions::new().read(true).write(true).open("/dev/random")?;
    if unsafe { ioctl(random.as_raw_fd(), 0x40085203, &pool as *const Pool) } != 0
        || unsafe { ioctl(random.as_raw_fd(), 0x5207, 0) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn read_identity(prepared: bool) -> std::io::Result<String> {
    let machine_id = fs::read_to_string("/etc/machine-id")?.trim().to_string();
    let hostname = fs::read_to_string("/etc/hostname")?.trim().to_string();
    let status = if prepared { "RNG-RESEEDED" } else { "UNPREPARED" };
    Ok(format!("IDENTITY {machine_id} {hostname} {status}"))
}

fn write_ram_mib(region: &mut [u8], mib: usize, value: u8) -> Result<(), &'static str> {
    let bytes = mib.checked_mul(1024 * 1024).ok_or("size overflow")?;
    if bytes > region.len() {
        return Err("requested RAM divergence exceeds probe region");
    }
    write_ram_pages(region, bytes / 4096, value)
}

fn write_ram_pages(region: &mut [u8], pages: usize, value: u8) -> Result<(), &'static str> {
    let max_pages = region.len() / 4096;
    if pages > max_pages {
        return Err("requested RAM divergence exceeds probe region");
    }
    for page in region[..pages * 4096].chunks_mut(4096) {
        page[0] = value;
    }
    Ok(())
}

fn main() {
    mount_fs("devtmpfs", "/dev", "devtmpfs");
    mount_fs("sysfs", "/sys", "sysfs");
    mount_fs("proc", "/proc", "proc");
    for module in ["failover", "net_failover", "virtio_net"] {
        load_module(module);
    }
    for _ in 0..100 {
        if File::open("/dev/vda").is_ok() && File::open("/dev/ttyS0").is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let mut serial_out = OpenOptions::new().write(true).open("/dev/ttyS0").expect("serial output");
    let serial_in = OpenOptions::new().read(true).open("/dev/ttyS0").expect("serial input");
    let mut reader = BufReader::new(serial_in);
    set_identity("00000000000000000000000000000000").expect("initialize template identity");
    let mut region = vec![0u8; 32 * 1024 * 1024];
    for page in region.chunks_mut(4096) {
        page[0] = 0x5a;
    }
    writeln!(serial_out, "DENSEVM READY").unwrap();
    let mut prepared = false;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            thread::sleep(Duration::from_millis(50));
            continue;
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        let result = match fields.as_slice() {
            ["ping"] => "PONG".to_string(),
            ["ram-read"] => format!("RAM {}", region[0]),
            ["ram-write", value] => match value.parse::<u8>() {
                Ok(value) => {
                    write_ram_mib(&mut region, 32, value).unwrap();
                    format!("RAM-WROTE {value}")
                }
                Err(_) => "ERROR invalid byte".to_string(),
            },
            ["ram-write-mib", mib, value] => match (mib.parse::<usize>(), value.parse::<u8>()) {
                (Ok(mib), Ok(value)) => match write_ram_mib(&mut region, mib, value) {
                    Ok(()) => format!("RAM-WROTE-MIB {mib} {value}"),
                    Err(error) => format!("ERROR {error}"),
                },
                _ => "ERROR invalid RAM write arguments".to_string(),
            },
            ["ram-write-pages", pages, value] => {
                match (pages.parse::<usize>(), value.parse::<u8>()) {
                    (Ok(pages), Ok(value)) => match write_ram_pages(&mut region, pages, value) {
                        Ok(()) => format!("RAM-WROTE-PAGES {pages} {value}"),
                        Err(error) => format!("ERROR {error}"),
                    },
                    _ => "ERROR invalid RAM write arguments".to_string(),
                }
            }
            ["disk-read"] => match disk_byte() {
                Ok(value) => format!("DISK {value}"),
                Err(error) => format!("ERROR disk read: {error}"),
            },
            ["disk-write", value] => match value.parse::<u8>() {
                Ok(value) => match write_disk(value) {
                    Ok(()) => format!("DISK-WROTE {value}"),
                    Err(error) => format!("ERROR disk write: {error}"),
                },
                Err(_) => "ERROR invalid byte".to_string(),
            },
            ["disk-fill", value] => match value.parse::<u8>() {
                Ok(value) => match fill_disk(value) {
                    Ok(()) => format!("DISK-FILLED {value}"),
                    Err(error) => format!("ERROR disk fill: {error}"),
                },
                Err(_) => "ERROR invalid byte".to_string(),
            },
            ["reboot"] => reboot_guest(),
            ["entropy-add", value] => match reseed(value) {
                Ok(()) => { prepared = true; "RNG-RESEEDED".to_string() },
                Err(error) => format!("ERROR reseed: {error}"),
            },
            ["identity-set", _] if !prepared => "ERROR reseed required before identity preparation".to_string(),
            ["identity-set", value] => match set_identity(value) {
                Ok(()) => format!("IDENTITY-SET {value}"),
                Err(error) => format!("ERROR identity set: {error}"),
            },
            ["identity-read"] => match read_identity(prepared) {
                Ok(value) => value,
                Err(error) => format!("ERROR identity read: {error}"),
            },
            ["mac-read"] => {
                let mut address = None;
                for _ in 0..50 {
                    if let Ok(value) = fs::read_to_string("/sys/class/net/eth0/address") {
                        address = Some(value);
                        break;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                match address {
                    Some(value) => format!("MAC {}", value.trim()),
                    None => "ERROR NIC did not appear".to_string(),
                }
            },
            _ => "ERROR unknown command".to_string(),
        };
        writeln!(serial_out, "DENSEVM {result}").unwrap();
        serial_out.flush().unwrap();
    }
}
