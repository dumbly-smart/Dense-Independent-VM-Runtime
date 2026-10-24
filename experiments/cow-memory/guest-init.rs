//! Tiny Linux guest used only to prove QEMU state restore and COW isolation.
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::raw::{c_char, c_int, c_long, c_ulong, c_void};
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static TIMER_PROBE_FD: AtomicI32 = AtomicI32::new(-1);
static TIMER_PROBE_EXPIRATIONS: AtomicU64 = AtomicU64::new(0);

#[repr(C)]
struct KernelTimeSpec {
    seconds: c_long,
    nanoseconds: c_long,
}

#[repr(C)]
struct KernelItimerSpec {
    interval: KernelTimeSpec,
    value: KernelTimeSpec,
}

unsafe extern "C" {
    fn mount(
        source: *const c_char,
        target: *const c_char,
        kind: *const c_char,
        flags: c_ulong,
        data: *const c_void,
    ) -> c_int;
    fn syscall(number: c_long, ...) -> c_long;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn sethostname(name: *const c_char, len: usize) -> c_int;
    fn socket(domain: c_int, socket_type: c_int, protocol: c_int) -> c_int;
    fn bind(fd: c_int, address: *const c_void, address_len: u32) -> c_int;
    fn poll(fds: *mut PollFd, count: usize, timeout_ms: c_int) -> c_int;
    fn recv(fd: c_int, buffer: *mut c_void, length: usize, flags: c_int) -> isize;
    fn close(fd: c_int) -> c_int;
    fn if_nametoindex(name: *const c_char) -> u32;
}

#[repr(C)]
struct SockAddrLl {
    family: u16,
    protocol: u16,
    interface_index: i32,
    hardware_type: u16,
    packet_type: u8,
    address_length: u8,
    address: [u8; 8],
}

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
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
    let code = unsafe {
        mount(
            source.as_ptr(),
            target.as_ptr(),
            kind.as_ptr(),
            0,
            std::ptr::null(),
        )
    };
    assert_eq!(code, 0, "mount failed: {}", std::io::Error::last_os_error());
}

fn disk_byte() -> std::io::Result<u8> {
    let mut file = File::open(disk_path())?;
    let mut byte = [0u8; 1];
    file.read_exact(&mut byte)?;
    Ok(byte[0])
}

fn write_disk(value: u8) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).open(disk_path())?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&[value; 4096])?;
    file.sync_all()
}

fn fill_disk(value: u8) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).open(disk_path())?;
    let block = [value; 1024 * 1024];
    for _ in 0..64 {
        file.write_all(&block)?;
    }
    file.sync_all()
}

fn disk_bench(iterations: &str) -> std::io::Result<String> {
    let iterations: usize = iterations.parse().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid disk benchmark iteration count",
        )
    })?;
    if !(1..=64).contains(&iterations) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "disk benchmark iterations must be 1..64",
        ));
    }
    if disk_path() == "/dev/vda" {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "disk benchmark requires a filesystem-backed guest",
        ));
    }
    let path = std::path::Path::new(disk_path()).with_file_name("disk-bench.bin");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?;
    let block = [0x5au8; 4096];
    let mut latencies = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        file.write_all(&block)?;
        file.sync_data()?;
        latencies.push(start.elapsed().as_micros());
    }
    latencies.sort_unstable();
    let mean_us = latencies.iter().sum::<u128>() / iterations as u128;
    let p50_us = latencies[iterations / 2];
    let p95_us = latencies[((iterations * 95).div_ceil(100))
        .saturating_sub(1)
        .min(iterations - 1)];
    Ok(format!(
        "DISK-BENCH iterations={iterations} block_bytes=4096 mean_us={mean_us} p50_us={p50_us} p95_us={p95_us}"
    ))
}

fn disk_path() -> &'static str {
    if std::path::Path::new("/etc/alpine-release").exists() {
        "/var/lib/densevm/disk-probe.bin"
    } else {
        "/dev/vda"
    }
}

fn services() -> std::io::Result<String> {
    let mut names = std::collections::HashSet::new();
    for entry in fs::read_dir("/proc")?.flatten() {
        if let Ok(name) = fs::read_to_string(entry.path().join("comm")) {
            names.insert(name.trim().to_owned());
        }
    }
    let init = fs::read_to_string("/proc/1/comm")?;
    Ok(format!(
        "SERVICES init={} syslogd={} crond={} getty={}",
        init.trim() == "init",
        names.contains("syslogd"),
        names.contains("crond"),
        names.contains("getty")
    ))
}

fn range_ip(value: &str) -> std::io::Result<std::net::Ipv4Addr> {
    let ip: std::net::Ipv4Addr = value.parse().map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid IPv4 address")
    })?;
    let octets = ip.octets();
    if octets[..3] != [10, 77, 0] || !(10..=41).contains(&octets[3]) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "address outside bounded range subnet",
        ));
    }
    Ok(ip)
}

fn network_set(value: &str) -> std::io::Result<String> {
    let ip = range_ip(value)?;
    for args in [
        vec![
            "ip".to_owned(),
            "addr".to_owned(),
            "add".to_owned(),
            format!("{ip}/24"),
            "dev".to_owned(),
            "eth0".to_owned(),
        ],
        vec![
            "ip".to_owned(),
            "link".to_owned(),
            "set".to_owned(),
            "eth0".to_owned(),
            "up".to_owned(),
        ],
    ] {
        let output = std::process::Command::new("/bin/busybox")
            .args(args)
            .output()?;
        if !output.status.success() {
            return Err(std::io::Error::other(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }
    }
    let socket = std::net::UdpSocket::bind((ip, 7777))?;
    let identity = fs::read_to_string("/etc/machine-id")?
        .trim()
        .as_bytes()
        .to_vec();
    thread::spawn(move || {
        let mut data = [0u8; 256];
        while let Ok((_, peer)) = socket.recv_from(&mut data) {
            let _ = socket.send_to(&identity, peer);
        }
    });
    Ok(format!("NETWORK {ip}"))
}

fn udp_probe(value: &str) -> std::io::Result<String> {
    let ip = range_ip(value)?;
    let socket = std::net::UdpSocket::bind("0.0.0.0:0")?;
    socket.set_read_timeout(Some(Duration::from_secs(3)))?;
    socket.send_to(b"densevm", (ip, 7777))?;
    let mut data = [0u8; 256];
    let (count, peer) = socket.recv_from(&mut data)?;
    if peer.ip() != std::net::IpAddr::V4(ip) {
        return Err(std::io::Error::other("unexpected UDP responder"));
    }
    Ok(format!("UDP {}", String::from_utf8_lossy(&data[..count])))
}

fn udp_bench(value: &str, packet_count: &str) -> std::io::Result<String> {
    const PACKET_BYTES: usize = 1024;
    let ip = range_ip(value)?;
    let packets: usize = packet_count.parse().map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid packet count")
    })?;
    if !(1..=128).contains(&packets) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "packet count must be 1..128",
        ));
    }
    let socket = std::net::UdpSocket::bind("0.0.0.0:0")?;
    socket.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut payload = [b'd'; PACKET_BYTES];
    let start = Instant::now();
    for sequence in 0..packets {
        payload[..4].copy_from_slice(&(sequence as u32).to_le_bytes());
        socket.send_to(&payload, (ip, 7777))?;
    }
    let mut received = 0usize;
    let mut response = [0u8; 256];
    let mut remote_identity = None;
    while received < packets {
        let (length, peer) = match socket.recv_from(&mut response) {
            Ok(reply) => reply,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(error) => return Err(error),
        };
        let value = String::from_utf8_lossy(&response[..length]).to_string();
        if peer.ip() != std::net::IpAddr::V4(ip)
            || value.len() != 32
            || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
            || remote_identity
                .as_ref()
                .is_some_and(|expected| expected != &value)
        {
            return Err(std::io::Error::other("unexpected UDP benchmark responder"));
        }
        remote_identity = Some(value);
        received += 1;
    }
    let elapsed_us = start.elapsed().as_micros().max(1);
    let offered_bytes = packets * PACKET_BYTES;
    let delivered_bytes = received * PACKET_BYTES;
    let packets_per_second = received as f64 * 1_000_000.0 / elapsed_us as f64;
    let payload_mbps = delivered_bytes as f64 * 8.0 / elapsed_us as f64;
    let loss_percent = (packets - received) as f64 * 100.0 / packets as f64;
    if remote_identity.is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "no UDP benchmark response",
        ));
    }
    Ok(format!(
        "UDP-BENCH sent_packets={packets} received_packets={received} offered_bytes={offered_bytes} delivered_bytes={delivered_bytes} elapsed_us={elapsed_us} packets_per_second={packets_per_second:.1} delivered_mbps={payload_mbps:.3} loss_percent={loss_percent:.2} remote_identity={}",
        remote_identity.unwrap_or_default()
    ))
}

fn process_count() -> std::io::Result<usize> {
    Ok(fs::read_dir("/proc")?
        .flatten()
        .filter(|entry| {
            fs::read(entry.path().join("cmdline")).is_ok_and(|cmd| cmd == b"/bin/sleep\x00137\x00")
        })
        .count())
}

fn reboot_guest() -> ! {
    let result = unsafe { syscall(169, 0xfee1dead_u32, 672274793_u32, 0x01234567_u32, 0_u32) };
    panic!("reboot syscall failed: {result}");
}

fn guest_oom_kill_count() -> Option<u64> {
    fs::read_to_string("/proc/vmstat")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("oom_kill "))
        .and_then(|value| value.parse().ok())
}

fn guest_clock() -> std::io::Result<String> {
    let realtime_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_millis();
    let uptime_seconds = fs::read_to_string("/proc/uptime")?
        .split_whitespace()
        .next()
        .ok_or_else(|| std::io::Error::other("/proc/uptime has no value"))?
        .parse::<f64>()
        .map_err(std::io::Error::other)?;
    Ok(format!(
        "CLOCK realtime_ms={realtime_ms} uptime_ms={:.0}",
        uptime_seconds * 1000.0
    ))
}

fn start_guest_timer_probe() -> std::io::Result<String> {
    if TIMER_PROBE_FD.load(Ordering::Acquire) >= 0 {
        return Ok("TIMER-STARTED already=true".to_string());
    }

    // x86-64 Linux syscall numbers: timerfd_create=283, timerfd_settime=286.
    let fd = unsafe { syscall(283, 1_i32, 0_i32) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let fd = fd as c_int;
    let timer = KernelItimerSpec {
        interval: KernelTimeSpec {
            seconds: 0,
            nanoseconds: 10_000_000,
        },
        value: KernelTimeSpec {
            seconds: 0,
            nanoseconds: 10_000_000,
        },
    };
    let configured = unsafe {
        syscall(
            286,
            fd,
            0_i32,
            &timer as *const KernelItimerSpec,
            std::ptr::null_mut::<KernelItimerSpec>(),
        )
    };
    if configured < 0 {
        unsafe { close(fd) };
        return Err(std::io::Error::last_os_error());
    }
    if TIMER_PROBE_FD
        .compare_exchange(-1, fd, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        unsafe { close(fd) };
        return Ok("TIMER-STARTED already=true".to_string());
    }

    thread::spawn(move || {
        loop {
            let mut expirations = 0_u64;
            let bytes_read = unsafe { syscall(0, fd, &mut expirations as *mut u64, 8_usize) };
            if bytes_read == 8 {
                TIMER_PROBE_EXPIRATIONS.fetch_add(expirations, Ordering::Relaxed);
            } else {
                break;
            }
        }
    });
    Ok("TIMER-STARTED period_ms=10".to_string())
}

fn guest_timer_expirations() -> String {
    if TIMER_PROBE_FD.load(Ordering::Acquire) < 0 {
        return "TIMER-UNKNOWN timer probe is not running".to_string();
    }
    format!(
        "TIMER-EXPIRATIONS count={}",
        TIMER_PROBE_EXPIRATIONS.load(Ordering::Relaxed)
    )
}

fn memory_pressure_worker() -> ! {
    let allocation_bytes = std::env::args()
        .nth(2)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if !(1..=128 * 1024 * 1024).contains(&allocation_bytes) {
        std::process::exit(43);
    }
    // Avoid reserving the whole range against the guest's commit limit. The
    // pages become real guest memory only as the worker touches them below.
    let memory = unsafe {
        mmap(
            std::ptr::null_mut(),
            allocation_bytes,
            0x1 | 0x2,
            0x02 | 0x20 | 0x4000,
            -1,
            0,
        )
    };
    if memory as isize == -1 {
        std::process::exit(42);
    }
    for offset in (0..allocation_bytes).step_by(4096) {
        unsafe { std::ptr::write_volatile(memory.cast::<u8>().add(offset), 0x5a) };
    }
    loop {
        std::hint::black_box(&memory);
        thread::sleep(Duration::from_secs(1));
    }
}

unsafe extern "C" {
    fn mmap(
        address: *mut std::ffi::c_void,
        length: usize,
        protection: i32,
        flags: i32,
        fd: i32,
        offset: i64,
    ) -> *mut std::ffi::c_void;
}

fn memory_pressure_probe() -> std::io::Result<String> {
    use std::os::unix::process::ExitStatusExt;

    let oom_before = guest_oom_kill_count();
    const WORKERS: usize = 8;
    const WORKER_BYTES: usize = 64 * 1024 * 1024;
    let executable = std::env::current_exe()?;
    let mut children = Vec::with_capacity(WORKERS);
    let mut spawn_failures = 0usize;
    for _ in 0..WORKERS {
        match std::process::Command::new(&executable)
            .args(["--memory-pressure-worker", &WORKER_BYTES.to_string()])
            .spawn()
        {
            Ok(child) => children.push(child),
            Err(_) => spawn_failures += 1,
        }
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut oom_killed_workers = 0usize;
    let mut allocation_failures = 0usize;
    let mut completed = vec![false; children.len()];
    loop {
        let mut live = 0usize;
        for (index, child) in children.iter_mut().enumerate() {
            if completed[index] {
                continue;
            }
            if let Some(status) = child.try_wait()? {
                completed[index] = true;
                if status.signal() == Some(9) {
                    oom_killed_workers += 1;
                } else if status.code() == Some(42) {
                    allocation_failures += 1;
                }
            } else {
                live += 1;
            }
        }
        let oom_delta = oom_before
            .zip(guest_oom_kill_count())
            .map(|(before, after)| after.saturating_sub(before));
        if oom_killed_workers > 0 && oom_delta.is_some_and(|delta| delta > 0) {
            for child in &mut children {
                if child.try_wait()?.is_none() {
                    let _ = child.kill();
                }
                let _ = child.wait();
            }
            return Ok(format!(
                "PRESSURE-OOM workers_killed={oom_killed_workers} oom_kill_delta={} allocation_failures={allocation_failures} spawn_failures={spawn_failures}",
                oom_delta.unwrap_or_default()
            ));
        }
        if live == 0 {
            return Ok(format!(
                "PRESSURE-NO-OOM oom_kill_delta={} allocation_failures={allocation_failures} spawn_failures={spawn_failures}",
                oom_delta.unwrap_or_default()
            ));
        }
        if Instant::now() >= deadline {
            for child in &mut children {
                if child.try_wait()?.is_none() {
                    let _ = child.kill();
                }
                let _ = child.wait();
            }
            return Ok(format!(
                "PRESSURE-TIMEOUT oom_kill_delta={} workers_killed={oom_killed_workers} allocation_failures={allocation_failures} spawn_failures={spawn_failures}",
                oom_delta.unwrap_or_default()
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn valid_identity(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn identity_preparation_allowed(prepared: bool) -> Result<(), &'static str> {
    if prepared {
        Ok(())
    } else {
        Err("guest entropy must be reseeded before identity or network setup")
    }
}

pub(crate) fn dhcpv4_client_identifier(frame: &[u8]) -> Option<Vec<u8>> {
    if frame.len() < 14 || frame.get(12..14)? != [0x08, 0x00] {
        return None;
    }
    let ip = frame.get(14..)?;
    if ip.len() < 20 || ip[0] >> 4 != 4 || ip[9] != 17 {
        return None;
    }
    let ip_header_bytes = usize::from(ip[0] & 0x0f) * 4;
    if ip_header_bytes < 20 || ip.len() < ip_header_bytes + 8 {
        return None;
    }
    let fragment = u16::from_be_bytes([ip[6], ip[7]]);
    if fragment & 0x1fff != 0 {
        return None;
    }
    let udp = ip.get(ip_header_bytes..)?;
    if udp.get(0..2)? != [0, 68] || udp.get(2..4)? != [0, 67] {
        return None;
    }
    let udp_len = usize::from(u16::from_be_bytes([udp[4], udp[5]]));
    if udp_len < 8 + 240 || udp.len() < udp_len {
        return None;
    }
    let dhcp = udp.get(8..udp_len)?;
    if dhcp.first().copied()? != 1 || dhcp.get(236..240)? != [99, 130, 83, 99] {
        return None;
    }
    let options = dhcp.get(240..)?;
    let mut offset = 0;
    while offset < options.len() {
        let option = options[offset];
        offset += 1;
        match option {
            0 => continue,
            255 => break,
            _ => {
                let length = usize::from(*options.get(offset)?);
                offset += 1;
                let value = options.get(offset..offset.checked_add(length)?)?;
                if option == 61 {
                    return Some(value.to_vec());
                }
                offset += length;
            }
        }
    }
    None
}

pub(crate) fn machine_id_client_identifier(value: &str) -> Result<Vec<u8>, &'static str> {
    if !valid_identity(value) {
        return Err("machine-id must contain 32 hexadecimal characters");
    }
    let mut identifier = Vec::with_capacity(17);
    identifier.push(0);
    for pair in value.as_bytes().chunks_exact(2) {
        let hex = std::str::from_utf8(pair).map_err(|_| "invalid machine-id hex")?;
        identifier.push(u8::from_str_radix(hex, 16).map_err(|_| "invalid machine-id hex")?);
    }
    Ok(identifier)
}

pub(crate) fn vmgenid_reseed_event_count(kernel_log: &str) -> usize {
    kernel_log
        .lines()
        .filter(|line| line.contains("crng reseeded due to virtual machine fork"))
        .count()
}

fn read_vmgenid_reseed_event_count() -> std::io::Result<usize> {
    let output = std::process::Command::new("/bin/busybox")
        .arg("dmesg")
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "dmesg failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(vmgenid_reseed_event_count(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn vmgenid_reaction_read() -> std::io::Result<String> {
    let baseline_path = "/var/lib/densevm/vmgenid-reseed-baseline";
    let baseline = match fs::read_to_string(baseline_path) {
        Ok(value) => value
            .trim()
            .parse::<usize>()
            .map_err(std::io::Error::other)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok("VMGENID-REACTION UNKNOWN template baseline unavailable".to_string());
        }
        Err(error) => return Err(error),
    };
    let current = read_vmgenid_reseed_event_count()?;
    let delta = current.saturating_sub(baseline);
    if delta > 0 {
        Ok(format!("VMGENID-REACTION VERIFIED event_delta={delta}"))
    } else {
        Ok(format!("VMGENID-REACTION UNKNOWN event_delta={delta}"))
    }
}

fn dhcp_packet_socket(interface: &str) -> std::io::Result<c_int> {
    let name = CString::new(interface).unwrap();
    let interface_index = unsafe { if_nametoindex(name.as_ptr()) };
    if interface_index == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let protocol = 0x0003u16.to_be();
    let fd = unsafe { socket(17, 3, c_int::from(protocol)) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let address = SockAddrLl {
        family: 17,
        protocol,
        interface_index: interface_index as i32,
        hardware_type: 0,
        packet_type: 0,
        address_length: 0,
        address: [0; 8],
    };
    let result = unsafe {
        bind(
            fd,
            (&address as *const SockAddrLl).cast(),
            std::mem::size_of::<SockAddrLl>() as u32,
        )
    };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        unsafe {
            close(fd);
        }
        return Err(error);
    }
    Ok(fd)
}

fn capture_dhcp_client_identifier(
    fd: c_int,
    expected: &[u8],
    timeout: Duration,
) -> std::io::Result<bool> {
    let deadline = Instant::now() + timeout;
    let mut frame = vec![0u8; 2048];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        let timeout_ms = remaining.as_millis().min(c_int::MAX as u128) as c_int;
        let mut poll_fd = PollFd {
            fd,
            events: 1,
            revents: 0,
        };
        let ready = unsafe { poll(&mut poll_fd, 1, timeout_ms) };
        if ready == 0 {
            return Ok(false);
        }
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        let length = unsafe { recv(fd, frame.as_mut_ptr().cast(), frame.len(), 0) };
        if length < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if dhcpv4_client_identifier(&frame[..length as usize]).as_deref() == Some(expected) {
            return Ok(true);
        }
    }
}

fn dhcp_identity_probe() -> std::io::Result<String> {
    if !std::path::Path::new("/etc/densevm-identity-validation").exists() {
        return Ok("DHCP-IDENTITY UNKNOWN test profile is not installed".to_string());
    }
    let machine_id = fs::read_to_string("/etc/machine-id")?;
    let machine_id = machine_id.trim();
    let client_id = machine_id_client_identifier(machine_id)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let client_id_hex = client_id
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let socket = dhcp_packet_socket("eth0")?;
    let script = "/var/lib/densevm/dhcp-noop.sh";
    fs::write(script, "#!/bin/sh\nexit 0\n")?;
    fs::set_permissions(script, fs::Permissions::from_mode(0o700))?;
    let option = format!("0x3d:{client_id_hex}");
    let output = std::process::Command::new("/bin/busybox")
        .args([
            "udhcpc", "-i", "eth0", "-f", "-n", "-q", "-t", "1", "-T", "2", "-x", &option, "-s",
            script,
        ])
        .output()?;
    let captured = capture_dhcp_client_identifier(socket, &client_id, Duration::from_secs(2));
    unsafe {
        close(socket);
    }
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "BusyBox udhcpc did not obtain a lease: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    if !captured? {
        return Err(std::io::Error::other(
            "DHCP packet did not contain the clone machine-id client identifier",
        ));
    }
    Ok(format!("DHCP-IDENTITY VERIFIED {client_id_hex}"))
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

fn ssh_host_key_paths() -> std::io::Result<Vec<std::path::PathBuf>> {
    let directory = std::path::Path::new("/etc/ssh");
    if !directory.exists() {
        return Ok(Vec::new());
    }
    Ok(fs::read_dir(directory)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("ssh_host_")
                        && (name.ends_with("_key") || name.ends_with("_key.pub"))
                })
        })
        .collect())
}

fn rotate_ssh_host_keys() -> std::io::Result<()> {
    let existing = ssh_host_key_paths()?;
    let server_installed = std::path::Path::new("/usr/sbin/sshd").exists();
    if existing.is_empty() && !server_installed {
        return Ok(());
    }
    for path in existing {
        fs::remove_file(path)?;
    }
    let output = std::process::Command::new("ssh-keygen")
        .arg("-A")
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "ssh-keygen -A failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    if server_installed && ssh_host_key_paths()?.is_empty() {
        return Err(std::io::Error::other(
            "sshd is installed but no host keys were regenerated",
        ));
    }
    Ok(())
}

fn ssh_host_key_identity() -> std::io::Result<String> {
    let paths = ssh_host_key_paths()?
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "pub"))
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Ok("SSH-IDENTITY ABSENT".to_string());
    }
    let mut fingerprints = Vec::new();
    for path in paths {
        let output = std::process::Command::new("ssh-keygen")
            .args(["-lf"])
            .arg(path)
            .output()?;
        if !output.status.success() {
            return Err(std::io::Error::other(format!(
                "ssh-keygen fingerprint failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let output = String::from_utf8_lossy(&output.stdout);
        let fingerprint = output
            .split_whitespace()
            .find(|field| field.starts_with("SHA256:"))
            .ok_or_else(|| std::io::Error::other("ssh-keygen returned no SHA256 fingerprint"))?;
        fingerprints.push(fingerprint.to_string());
    }
    fingerprints.sort();
    Ok(format!("SSH-IDENTITY {}", fingerprints.join(",")))
}

fn reseed(value: &str) -> std::io::Result<()> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "expected 32 bytes of host entropy",
        ));
    }
    // Linux UAPI linux/random.h: rand_pool_info and RNDADDENTROPY,
    // followed by RNDRESEEDCRNG to force a new CRNG key before networking.
    #[repr(C)]
    struct Pool {
        entropy_count: c_int,
        size: c_int,
        bytes: [u8; 32],
    }
    let mut pool = Pool {
        entropy_count: 256,
        size: 32,
        bytes: [0; 32],
    };
    for (i, byte) in pool.bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).unwrap();
    }
    let random = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/random")?;
    if unsafe { ioctl(random.as_raw_fd(), 0x40085203, &pool as *const Pool) } != 0
        || unsafe { ioctl(random.as_raw_fd(), 0x5207, 0) } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn read_identity(prepared: bool) -> std::io::Result<String> {
    let machine_id = fs::read_to_string("/etc/machine-id")?.trim().to_string();
    let hostname = fs::read_to_string("/etc/hostname")?.trim().to_string();
    let status = if prepared {
        "RNG-RESEEDED"
    } else {
        "UNPREPARED"
    };
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

fn random_ram_pages(region: &mut [u8], pages: usize, value: u8) -> std::io::Result<()> {
    let bytes = pages
        .checked_mul(4096)
        .filter(|bytes| *bytes <= region.len())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "RAM divergence exceeds region",
            )
        })?;
    File::open("/dev/urandom")?.read_exact(&mut region[..bytes])?;
    if bytes > 0 {
        region[0] = value;
    }
    Ok(())
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--memory-pressure-worker") {
        memory_pressure_worker();
    }
    if !std::path::Path::new("/dev/null").exists() {
        mount_fs("devtmpfs", "/dev", "devtmpfs");
    }
    if !std::path::Path::new("/sys/class").exists() {
        mount_fs("sysfs", "/sys", "sysfs");
    }
    if !std::path::Path::new("/proc/self").exists() {
        mount_fs("proc", "/proc", "proc");
    }
    for module in ["failover", "net_failover", "virtio_net"] {
        load_module(module);
    }
    for _ in 0..100 {
        if File::open("/dev/vda").is_ok() && File::open("/dev/ttyS0").is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let mut serial_out = OpenOptions::new()
        .write(true)
        .open("/dev/ttyS0")
        .expect("serial output");
    let serial_in = OpenOptions::new()
        .read(true)
        .open("/dev/ttyS0")
        .expect("serial input");
    let mut reader = BufReader::new(serial_in);
    set_identity("00000000000000000000000000000000").expect("initialize template identity");
    if std::path::Path::new("/etc/alpine-release").exists() {
        let event_count = read_vmgenid_reseed_event_count().expect("read template VMGenID log");
        fs::write(
            "/var/lib/densevm/vmgenid-reseed-baseline",
            event_count.to_string(),
        )
        .expect("save template VMGenID log baseline");
    }
    let mut region = vec![0u8; 32 * 1024 * 1024];
    for page in region.chunks_mut(4096) {
        page[0] = 0x5a;
    }
    writeln!(serial_out, "DENSEVM READY").unwrap();
    let mut prepared = false;
    let mut processes = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            thread::sleep(Duration::from_millis(50));
            continue;
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        let result = match fields.as_slice() {
            ["ping"] => "PONG".to_string(),
            ["clock-read"] => match guest_clock() {
                Ok(value) => value,
                Err(error) => format!("ERROR guest clock: {error}"),
            },
            ["timer-start"] => match start_guest_timer_probe() {
                Ok(value) => value,
                Err(error) => format!("ERROR timer start: {error}"),
            },
            ["timer-read"] => guest_timer_expirations(),
            ["sync"] => {
                unsafe {
                    syscall(162);
                }
                "SYNCED".to_string()
            }
            ["services-read"] => match services() {
                Ok(value) => value,
                Err(e) => format!("ERROR services: {e}"),
            },
            ["network-set", value] => match identity_preparation_allowed(prepared) {
                Err(_) => "ERROR identity preparation required before networking".to_string(),
                Ok(()) => match network_set(value) {
                    Ok(value) => value,
                    Err(e) => format!("ERROR network: {e}"),
                },
            },
            ["udp-probe", value] => match udp_probe(value) {
                Ok(value) => value,
                Err(e) => format!("ERROR UDP: {e}"),
            },
            ["udp-bench", value, packets] => match udp_bench(value, packets) {
                Ok(value) => value,
                Err(e) => format!("ERROR UDP benchmark: {e}"),
            },
            ["process-start"] => {
                match std::process::Command::new("/bin/sleep").arg("137").spawn() {
                    Ok(child) => {
                        processes.push(child);
                        "PROCESS-STARTED".to_string()
                    }
                    Err(e) => format!("ERROR process: {e}"),
                }
            }
            ["process-count"] => match process_count() {
                Ok(n) => format!("PROCESS-COUNT {n}"),
                Err(e) => format!("ERROR process: {e}"),
            },
            ["memory-pressure"] => match memory_pressure_probe() {
                Ok(value) => value,
                Err(error) => format!("ERROR memory pressure: {error}"),
            },
            ["kernel-crash"] => {
                if let Err(error) = fs::write("/proc/sys/kernel/sysrq", "1") {
                    format!("ERROR enable SysRq: {error}")
                } else {
                    writeln!(serial_out, "DENSEVM KERNEL-CRASH-ARMED").unwrap();
                    match fs::write("/proc/sysrq-trigger", "c") {
                        Ok(()) => "ERROR SysRq crash trigger returned".to_string(),
                        Err(error) => format!("ERROR trigger kernel crash: {error}"),
                    }
                }
            }
            ["cpu-burn", millis] => match millis.parse::<u64>() {
                Ok(millis) if (1..=10_000).contains(&millis) => {
                    let deadline = Instant::now() + Duration::from_millis(millis);
                    let mut value = 0x9e3779b97f4a7c15u64;
                    let mut iterations = 0u64;
                    while Instant::now() < deadline {
                        value = value.rotate_left(7).wrapping_mul(0x517cc1b727220a95);
                        iterations = iterations.wrapping_add(1);
                        std::hint::black_box(value);
                    }
                    format!("CPU-BURN {iterations}")
                }
                _ => "ERROR cpu-burn duration must be 1..10000 ms".to_string(),
            },
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
            ["ram-random-pages", pages, value] => {
                match (pages.parse::<usize>(), value.parse::<u8>()) {
                    (Ok(pages), Ok(value)) => match random_ram_pages(&mut region, pages, value) {
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
            ["disk-bench", iterations] => match disk_bench(iterations) {
                Ok(value) => value,
                Err(error) => format!("ERROR disk benchmark: {error}"),
            },
            ["reboot"] => {
                writeln!(serial_out, "DENSEVM REBOOTING").unwrap();
                serial_out.flush().unwrap();
                reboot_guest()
            }
            ["entropy-add", value] => match reseed(value) {
                Ok(()) => {
                    prepared = true;
                    "RNG-RESEEDED".to_string()
                }
                Err(error) => format!("ERROR reseed: {error}"),
            },
            ["identity-set", value] => match identity_preparation_allowed(prepared) {
                Err(error) => format!("ERROR {error}"),
                Ok(()) => match set_identity(value) {
                    Ok(()) => match rotate_ssh_host_keys() {
                        Ok(()) => format!("IDENTITY-SET {value}"),
                        Err(error) => format!("ERROR SSH host key rotation: {error}"),
                    },
                    Err(error) => format!("ERROR identity set: {error}"),
                },
            },
            ["identity-read"] => match read_identity(prepared) {
                Ok(value) => value,
                Err(error) => format!("ERROR identity read: {error}"),
            },
            ["ssh-identity-read"] => match ssh_host_key_identity() {
                Ok(value) => value,
                Err(error) => format!("ERROR SSH identity read: {error}"),
            },
            ["dhcp-identity-read"] => {
                "DHCP-IDENTITY UNKNOWN DHCP client identity is not exercised by this guest"
                    .to_string()
            }
            ["dhcp-identity-probe"] => match dhcp_identity_probe() {
                Ok(value) => value,
                Err(error) => format!("ERROR DHCP identity: {error}"),
            },
            ["vmgenid-reaction-read"] => match vmgenid_reaction_read() {
                Ok(value) => value,
                Err(error) => format!("ERROR VMGenID reaction: {error}"),
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
            }
            _ => "ERROR unknown command".to_string(),
        };
        writeln!(serial_out, "DENSEVM {result}").unwrap();
        serial_out.flush().unwrap();
    }
}
