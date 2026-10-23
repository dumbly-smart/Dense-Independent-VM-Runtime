use crate::identity::CloneIdentity;
use crate::metrics::{MemoryTotals, parse_smaps_for_path, parse_smaps_rollup};
use crate::qmp::Qmp;
use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const RAM_MIB: u64 = 256;
const RAM_BYTES: u64 = RAM_MIB * 1024 * 1024;
const TEST_REGION_MIB: u32 = 32;
const TEST_REGION_PAGES: u32 = TEST_REGION_MIB * 256;
const ACTIVE_CPU_BURN_MS: u64 = 1500;

struct Vm {
    child: Child,
    qmp_path: PathBuf,
    serial_path: PathBuf,
    command: Vec<String>,
    login_stream: Option<UnixStream>,
}

impl Vm {
    fn pid(&self) -> u32 {
        self.child.id()
    }
    fn qmp(&self) -> Result<Qmp, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match Qmp::connect(&self.qmp_path, Duration::from_secs(10)) {
                Ok(qmp) => return Ok(qmp),
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    thread::sleep(Duration::from_millis(50));
                }
                Err(error) => return Err(error),
            }
        }
    }
    fn serial(&self) -> Result<Serial, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match UnixStream::connect(&self.serial_path) {
                Ok(stream) => return Serial::new(stream),
                Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                Err(error) => return Err(format!("serial connect: {error}")),
            }
        }
    }
}

impl Drop for Vm {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Serial {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Serial {
    fn new(stream: UnixStream) -> Result<Self, String> {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;
        let writer = stream.try_clone().map_err(|e| e.to_string())?;
        Ok(Self {
            reader: BufReader::new(stream),
            writer,
        })
    }
    fn expect(&mut self, wanted: &str, timeout: Duration) -> Result<String, String> {
        let deadline = Instant::now() + timeout;
        loop {
            crate::interrupt::check()?;
            if Instant::now() >= deadline {
                return Err(format!("serial timeout waiting for {wanted}"));
            }
            let mut line = String::new();
            match self.reader.read_line(&mut line) {
                Ok(0) => thread::sleep(Duration::from_millis(20)),
                Ok(_) => {
                    if let Some(result) = line.trim().strip_prefix("DENSEVM ") {
                        if result.starts_with(wanted) {
                            return Ok(result.to_string());
                        }
                        if result.starts_with("ERROR") {
                            return Err(result.to_string());
                        }
                    }
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => return Err(format!("serial read: {error}")),
            }
        }
    }
    fn command(&mut self, command: &str, wanted: &str) -> Result<String, String> {
        self.send(command)?;
        self.expect(wanted, Duration::from_secs(12))
    }
    fn send(&mut self, command: &str) -> Result<(), String> {
        crate::interrupt::check()?;
        writeln!(self.writer, "{command}").map_err(|e| format!("serial write: {e}"))?;
        self.writer.flush().map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn checked(command: &mut Command) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|e| format!("spawn {:?}: {e}", command))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("command {:?} exited {status}", command))
    }
}

fn output(command: &mut Command) -> Result<String, String> {
    let result = command
        .output()
        .map_err(|e| format!("spawn {:?}: {e}", command))?;
    if !result.status.success() {
        return Err(format!(
            "command {:?} exited {}: {}",
            command,
            result.status,
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    String::from_utf8(result.stdout).map_err(|e| e.to_string())
}

fn sha256(path: &Path) -> Result<String, String> {
    let result = output(Command::new("sha256sum").arg(path))?;
    result
        .split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| "empty sha256sum output".to_string())
}

fn build_initramfs(out: &Path) -> Result<PathBuf, String> {
    let root = out.join("initramfs-root");
    fs::create_dir_all(root.join("dev")).map_err(|e| e.to_string())?;
    fs::create_dir_all(root.join("modules")).map_err(|e| e.to_string())?;
    for module in ["failover", "net_failover", "virtio_net"] {
        let filename = output(Command::new("modinfo").args(["-F", "filename", module]))?;
        let source = Path::new(filename.trim());
        fs::copy(source, root.join(format!("modules/{module}.ko.zst")))
            .map_err(|e| format!("copy {module}: {e}"))?;
    }
    let guest = Path::new(env!("CARGO_MANIFEST_DIR")).join("experiments/cow-memory/guest-init.rs");
    checked(
        Command::new("rustc")
            .arg("--edition=2024")
            .arg("-C")
            .arg("target-feature=+crt-static")
            .arg("-C")
            .arg("panic=abort")
            .arg("-O")
            .arg(&guest)
            .arg("-o")
            .arg(root.join("init")),
    )?;
    let archive = out.join("initramfs.cpio");
    let mut process = Command::new("cpio")
        .args(["-o", "-H", "newc", "--quiet"])
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(
            File::create(&archive).map_err(|e| e.to_string())?,
        ))
        .spawn()
        .map_err(|e| e.to_string())?;
    process.stdin.take().unwrap().write_all(b"./dev\n./modules\n./modules/failover.ko.zst\n./modules/net_failover.ko.zst\n./modules/virtio_net.ko.zst\n./init\n").map_err(|e| e.to_string())?;
    let status = process.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("cpio exited {status}"));
    }
    Ok(archive)
}

enum RamBackend<'a> {
    Anonymous,
    File { path: &'a Path, source: bool },
}

struct GuestArtifacts {
    kernel: PathBuf,
    initramfs: PathBuf,
    disk: Option<PathBuf>,
    metadata: Value,
    netdev: Option<String>,
    merge: bool,
}

fn prepare_guest(out: &Path, guest_dir: Option<&Path>) -> Result<GuestArtifacts, String> {
    if let Some(directory) = guest_dir {
        let directory = directory
            .canonicalize()
            .map_err(|e| format!("guest directory: {e}"))?;
        let metadata: Value = serde_json::from_slice(
            &fs::read(directory.join("metadata.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if metadata["kind"] != "alpine-ext4-v1" {
            return Err("unsupported prepared guest kind".into());
        }
        for (file, key) in [
            ("kernel", "kernel_sha256"),
            ("initramfs.cpio", "initramfs_sha256"),
            ("disk.qcow2", "disk_sha256"),
        ] {
            if metadata[key].as_str() != Some(sha256(&directory.join(file))?.as_str()) {
                return Err(format!("prepared guest {file} hash mismatch"));
            }
        }
        Ok(GuestArtifacts {
            kernel: directory.join("kernel"),
            initramfs: directory.join("initramfs.cpio"),
            disk: Some(directory.join("disk.qcow2")),
            metadata,
            netdev: None,
            merge: false,
        })
    } else {
        Ok(GuestArtifacts {
            kernel: PathBuf::from("/boot/vmlinuz-linux"),
            initramfs: build_initramfs(out)?,
            disk: None,
            metadata: json!({"kind":"synthetic-initramfs"}),
            netdev: None,
            merge: false,
        })
    }
}

fn create_base(base: &Path, guest: &GuestArtifacts) -> Result<(), String> {
    if let Some(disk) = &guest.disk {
        checked(
            Command::new("qemu-img")
                .args(["convert", "-f", "qcow2", "-O", "qcow2"])
                .arg(disk)
                .arg(base),
        )
    } else {
        checked(
            Command::new("qemu-img")
                .args(["create", "-f", "qcow2"])
                .arg(base)
                .arg("64M"),
        )
    }
}

fn login_prompt(stream: &mut UnixStream) -> Result<bool, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    let mut bytes = [0u8; 1024];
    let mut text = String::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && text.len() < 4096 {
        crate::interrupt::check()?;
        match stream.read(&mut bytes) {
            Ok(0) => break,
            Ok(n) => {
                text.push_str(&String::from_utf8_lossy(&bytes[..n]));
                if text.contains("login:") {
                    return Ok(true);
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Err(format!(
        "serial login prompt not observed; captured {text:?}"
    ))
}

#[cfg(test)]
mod login_tests {
    use super::*;
    #[test]
    fn accepts_a_prompt_split_across_many_serial_reads() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let writer = thread::spawn(move || {
            for byte in b"\r\nWelcome to Linux\r\nhost login:" {
                if server.write_all(&[*byte]).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(2));
            }
        });
        let result = login_prompt(&mut client);
        drop(client);
        writer.join().unwrap();
        assert_eq!(result.unwrap(), true);
    }

    #[test]
    fn converts_scheduler_ticks_to_single_core_cpu_percent() {
        assert_eq!(cpu_ticks_as_percent_of_one_core(100, 1000, 100), 100.0);
        assert_eq!(cpu_ticks_as_percent_of_one_core(50, 1000, 100), 50.0);
        assert_eq!(cpu_ticks_as_percent_of_one_core(100, 0, 100), 0.0);
        assert_eq!(cpu_ticks_as_percent_of_one_core(100, 1000, 0), 0.0);
    }
}

fn launch(
    out: &Path,
    name: &str,
    disk: &Path,
    backend: RamBackend<'_>,
    guest: &GuestArtifacts,
    mac_at_boot: Option<&str>,
    incoming: bool,
) -> Result<Vm, String> {
    let qmp_path = out.join(format!("{name}.qmp.sock"));
    let serial_path = out.join(format!("{name}.serial.sock"));
    let log = File::create(out.join(format!("{name}.qemu.log"))).map_err(|e| e.to_string())?;
    let mut cmd = Command::new("qemu-system-x86_64");
    cmd.args([
        "-machine",
        &format!(
            "{},accel=kvm,mem-merge={}",
            crate::capabilities::MACHINE,
            if guest.merge { "on" } else { "off" }
        ),
        "-cpu",
        "host",
        "-m",
        "256M",
        "-smp",
        "1",
    ]);
    if let RamBackend::File { path, source } = backend {
        let opts = if source {
            format!(
                "memory-backend-file,id=ram,mem-path={},size=256M,share=on,readonly=off",
                path.display()
            )
        } else {
            format!(
                "memory-backend-file,id=ram,mem-path={},size=256M,share=off,readonly=on,rom=off",
                path.display()
            )
        };
        let opts = format!("{opts},merge={}", if guest.merge { "on" } else { "off" });
        cmd.args(["-object", &opts, "-machine", "memory-backend=ram"]);
    }
    cmd.arg("-kernel")
        .arg(&guest.kernel)
        .arg("-initrd")
        .arg(&guest.initramfs)
        .args([
            "-append",
            "console=ttyS0 rdinit=/init panic=-1 loglevel=3",
            "-display",
            "none",
            "-monitor",
            "none",
            "-nodefaults",
            "-no-reboot",
        ]);
    cmd.arg("-drive").arg(format!(
        "if=none,id=disk,file={},format=qcow2",
        disk.display()
    ));
    cmd.args(["-device", "virtio-blk-pci,drive=disk,id=blk0", "-netdev"]);
    cmd.arg(
        guest
            .netdev
            .as_deref()
            .unwrap_or("user,id=net0,restrict=on"),
    );
    cmd.args(["-device", "pcie-root-port,id=rp1,slot=2,chassis=1"]);
    if let Some(mac) = mac_at_boot {
        cmd.arg("-device").arg(format!(
            "virtio-net-pci,bus=rp1,netdev=net0,mac={mac},id=nic0"
        ));
    }
    cmd.args(["-device", "vmgenid,guid=auto"]);
    cmd.arg("-chardev").arg(format!(
        "socket,id=serial,path={},server=on,wait=off",
        serial_path.display()
    ));
    cmd.args(["-serial", "chardev:serial"]);
    if guest.disk.is_some() {
        cmd.arg("-chardev").arg(format!(
            "socket,id=login,path={},server=on,wait=on",
            out.join(format!("{name}.login.sock")).display()
        ));
        cmd.args(["-serial", "chardev:login"]);
    }
    cmd.arg("-qmp")
        .arg(format!("unix:{},server=on,wait=off", qmp_path.display()));
    if incoming {
        cmd.args(["-incoming", "defer"]);
    }
    let command = std::iter::once("qemu-system-x86_64".to_string())
        .chain(cmd.get_args().map(|arg| arg.to_string_lossy().into_owned()))
        .collect();
    let child = cmd
        .stderr(Stdio::from(log))
        .stdout(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("launch {name}: {e}"))?;
    let mut vm = Vm {
        child,
        qmp_path,
        serial_path,
        command,
        login_stream: None,
    };
    if guest.disk.is_some() {
        let login = out.join(format!("{name}.login.sock"));
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            crate::interrupt::check()?;
            match UnixStream::connect(&login) {
                Ok(stream) => {
                    vm.login_stream = Some(stream);
                    break;
                }
                Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                Err(error) => return Err(format!("connect login console: {error}")),
            }
        }
    }
    Ok(vm)
}

fn measure(pid: u32) -> Result<MemoryTotals, String> {
    let text =
        fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).map_err(|e| e.to_string())?;
    parse_smaps_rollup(&text)
}

fn process_cpu_ticks(pid: u32) -> Result<u64, String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).map_err(|e| e.to_string())?;
    let (_, tail) = stat
        .rsplit_once(')')
        .ok_or_else(|| format!("malformed /proc/{pid}/stat"))?;
    let fields: Vec<_> = tail.split_whitespace().collect();
    let user = fields
        .get(11)
        .ok_or_else(|| format!("missing user CPU time in /proc/{pid}/stat"))?
        .parse::<u64>()
        .map_err(|e| e.to_string())?;
    let system = fields
        .get(12)
        .ok_or_else(|| format!("missing system CPU time in /proc/{pid}/stat"))?
        .parse::<u64>()
        .map_err(|e| e.to_string())?;
    Ok(user + system)
}

fn sample_cpu_ticks(pids: &[u32]) -> Result<(u128, Vec<u64>), String> {
    let before = pids
        .iter()
        .map(|pid| process_cpu_ticks(*pid))
        .collect::<Result<Vec<_>, _>>()?;
    let start = Instant::now();
    thread::sleep(Duration::from_secs(1));
    let elapsed_ms = start.elapsed().as_millis();
    let after = pids
        .iter()
        .map(|pid| process_cpu_ticks(*pid))
        .collect::<Result<Vec<_>, _>>()?;
    let deltas = before
        .into_iter()
        .zip(after)
        .map(|(before, after)| after.saturating_sub(before))
        .collect();
    Ok((elapsed_ms, deltas))
}

fn sample_active_cpu_ticks(
    pids: &[u32],
    serials: &mut [&mut Serial],
) -> Result<(u128, Vec<u64>), String> {
    if pids.len() != serials.len() {
        return Err("active CPU sample must include one serial endpoint per VM".into());
    }
    for serial in serials.iter_mut() {
        serial.send(&format!("cpu-burn {ACTIVE_CPU_BURN_MS}"))?;
    }
    let sample = sample_cpu_ticks(pids)?;
    for serial in serials.iter_mut() {
        serial.expect("CPU-BURN ", Duration::from_secs(5))?;
    }
    Ok(sample)
}

fn cpu_ticks_as_percent_of_one_core(ticks: u64, interval_ms: u128, ticks_per_second: u64) -> f64 {
    if interval_ms == 0 || ticks_per_second == 0 {
        return 0.0;
    }
    ticks as f64 * 100_000.0 / (interval_ms as f64 * ticks_per_second as f64)
}

fn measure_mapping(pid: u32, path: &Path) -> Result<MemoryTotals, String> {
    let text = fs::read_to_string(format!("/proc/{pid}/smaps")).map_err(|e| e.to_string())?;
    crate::metrics::verify_private_ram_mapping(&text, path.to_str().ok_or("non-UTF8 RAM path")?)?;
    parse_smaps_for_path(&text, path.to_str().ok_or("non-UTF8 RAM path")?)
}

fn snapshot(pids: &[u32], ram: &Path) -> Result<Value, String> {
    let mut clones = Vec::new();
    for pid in pids {
        clones.push(
            json!({"pid":pid,"memory":measure(*pid)?,"ram_mapping":measure_mapping(*pid, ram)?}),
        );
    }
    Ok(
        json!({"clones":clones,"template_file_resident_bytes":resident_file_bytes(ram)?,"sampling":"sequential procfs reads while all clones are alive"}),
    )
}

fn allocated_bytes(path: &Path) -> Result<u64, String> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path)
        .map(|x| x.blocks() * 512)
        .map_err(|e| e.to_string())
}

fn ksmd_cpu_ticks() -> Option<u64> {
    let entries = fs::read_dir("/proc").ok()?;
    let mut total = 0u64;
    let mut found = false;
    for entry in entries.flatten() {
        let pid = entry.file_name().to_string_lossy().parse::<u32>().ok();
        let Some(pid) = pid else { continue };
        let proc_dir = PathBuf::from("/proc").join(pid.to_string());
        let Ok(comm) = fs::read_to_string(proc_dir.join("comm")) else {
            continue;
        };
        if comm.trim() != "ksmd" {
            continue;
        }
        let Ok(stat) = fs::read_to_string(proc_dir.join("stat")) else {
            continue;
        };
        let Some((_, tail)) = stat.rsplit_once(')') else {
            continue;
        };
        let fields: Vec<_> = tail.split_whitespace().collect();
        let (Some(user), Some(system)) = (fields.get(11), fields.get(12)) else {
            continue;
        };
        let (Ok(user_ticks), Ok(system_ticks)) = (user.parse::<u64>(), system.parse::<u64>())
        else {
            continue;
        };
        total += user_ticks + system_ticks;
        found = true;
    }
    found.then_some(total)
}

fn resident_file_bytes(path: &Path) -> Result<u64, String> {
    let raw = output(
        Command::new("fincore")
            .args(["--json", "--bytes", "--output", "FILE,SIZE,RES"])
            .arg(path),
    )?;
    let data: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    data["fincore"][0]["res"]
        .as_u64()
        .ok_or_else(|| format!("invalid fincore output: {data}"))
}

fn verify_backing(overlay: &Path, base: &Path) -> Result<(), String> {
    let raw = output(
        Command::new("qemu-img")
            .args(["info", "--output=json", "--backing-chain"])
            .arg(overlay),
    )?;
    let chain: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    if chain[0]["filename"] != overlay.to_string_lossy().as_ref()
        || chain[0]["format"] != "qcow2"
        || chain[0]["full-backing-filename"] != base.to_string_lossy().as_ref()
        || chain[0]["backing-filename-format"] != "qcow2"
        || chain[1]["filename"] != base.to_string_lossy().as_ref()
        || chain[1]["format"] != "qcow2"
    {
        return Err(format!("overlay backing chain mismatch: {chain}"));
    }
    Ok(())
}

fn wait_migration(qmp: &mut Qmp) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = qmp.execute("query-migrate", None)?;
        match state["status"].as_str() {
            Some("completed") => return Ok(state),
            Some("failed" | "cancelled") => return Err(format!("migration failed: {state}")),
            _ if Instant::now() >= deadline => return Err(format!("migration timeout: {state}")),
            _ => thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn enable_ignore_shared(qmp: &mut Qmp) -> Result<(), String> {
    let capabilities = qmp.execute("query-migrate-capabilities", None)?;
    let supported = capabilities.as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["capability"] == "x-ignore-shared")
    });
    if !supported {
        return Err("this QEMU lacks x-ignore-shared".into());
    }
    qmp.execute(
        "migrate-set-capabilities",
        Some(json!({"capabilities":[{"capability":"x-ignore-shared","state":true}]})),
    )?;
    Ok(())
}

fn restore(qmp: &mut Qmp, state: &Path) -> Result<Value, String> {
    enable_ignore_shared(qmp)?;
    qmp.execute(
        "migrate-incoming",
        Some(json!({"uri":format!("file:{}", state.display())})),
    )?;
    let result = wait_migration(qmp)?;
    if qmp.execute("query-status", None)?["status"] == "paused" {
        qmp.execute("cont", None)?;
    }
    Ok(result)
}

fn add_nic(qmp: &mut Qmp, mac: &str) -> Result<(), String> {
    qmp.execute(
        "device_add",
        Some(json!({"driver":"virtio-net-pci","bus":"rp1","netdev":"net0","mac":mac,"id":"nic0"})),
    )?;
    Ok(())
}

fn stop_and_verify(qmp: &mut Qmp) -> Result<bool, String> {
    qmp.execute("stop", None)?;
    Ok(qmp.execute("query-status", None)?["status"] == "paused")
}

fn resume_and_verify(qmp: &mut Qmp) -> Result<bool, String> {
    qmp.execute("cont", None)?;
    Ok(qmp.execute("query-status", None)?["status"] == "running")
}

fn expect_value(serial: &mut Serial, command: &str, wanted: &str) -> Result<bool, String> {
    Ok(serial.command(command, wanted)? == wanted)
}

pub fn run(out: &Path) -> Result<(), String> {
    run_with_count(out, 2)
}

pub fn run_with_count(out: &Path, count: u32) -> Result<(), String> {
    run_with_options(out, count, false)
}

pub fn run_with_options(out: &Path, count: u32, ksm: bool) -> Result<(), String> {
    run_with_options_and_divergence(out, count, ksm, TEST_REGION_PAGES)
}

pub fn run_with_options_and_divergence(
    out: &Path,
    count: u32,
    ksm: bool,
    divergence_pages: u32,
) -> Result<(), String> {
    run_with_options_and_divergence_and_parking(out, count, ksm, divergence_pages, false)
}

pub fn run_with_options_and_divergence_and_parking(
    out: &Path,
    count: u32,
    ksm: bool,
    divergence_pages: u32,
    park_idle: bool,
) -> Result<(), String> {
    run_experiment(out, count, ksm, divergence_pages, park_idle, false)
}

pub fn run_experiment(
    out: &Path,
    count: u32,
    ksm: bool,
    divergence_pages: u32,
    park_idle: bool,
    divergence_all: bool,
) -> Result<(), String> {
    run_options(
        out,
        ProbeOptions {
            count,
            ksm,
            divergence_pages,
            park_idle,
            divergence_all,
            guest_dir: None,
            range: false,
        },
    )
}

#[derive(Clone, Copy)]
pub struct ProbeOptions<'a> {
    pub count: u32,
    pub ksm: bool,
    pub divergence_pages: u32,
    pub park_idle: bool,
    pub divergence_all: bool,
    pub guest_dir: Option<&'a Path>,
    pub range: bool,
}

pub fn run_options(out: &Path, options: ProbeOptions<'_>) -> Result<(), String> {
    let ProbeOptions {
        count,
        ksm,
        divergence_pages,
        ..
    } = options;
    if !(2..=32).contains(&count) {
        return Err(
            "the template restore probe supports clone counts from 2 through 32".to_string(),
        );
    }
    if divergence_pages > TEST_REGION_PAGES {
        return Err(format!(
            "RAM divergence must be at most {TEST_REGION_PAGES} pages"
        ));
    }
    if options.range && (options.guest_dir.is_none() || count < 3) {
        return Err("--range requires a prepared Alpine guest and at least 3 clones".into());
    }
    let mut ksm_guard = if ksm {
        Some(crate::ksm::KsmGuard::enable()?)
    } else {
        None
    };
    let result = run_two_clone(out, options);
    if let Some(guard) = &mut ksm_guard {
        guard.restore()?;
    }
    result
}

fn run_two_clone(out: &Path, options: ProbeOptions<'_>) -> Result<(), String> {
    let ProbeOptions {
        count,
        ksm,
        divergence_pages,
        park_idle,
        divergence_all,
        guest_dir,
        range,
    } = options;
    let ksm_counters_before = crate::inspect::ksm_status().ok();
    let host = crate::capabilities::host_report();
    crate::capabilities::require_template_support(&host)?;
    let clone_identities = (0..count)
        .map(|_| CloneIdentity::generate())
        .collect::<Result<Vec<_>, _>>()?;
    let ids = clone_identities
        .iter()
        .map(|id| &id.machine_id)
        .collect::<std::collections::HashSet<_>>();
    let macs = clone_identities
        .iter()
        .map(|id| &id.mac)
        .collect::<std::collections::HashSet<_>>();
    if ids.len() != count as usize || macs.len() != count as usize {
        return Err(
            "host RNG returned duplicate clone identities; refusing network attachment".into(),
        );
    }
    let ksmd_cpu_ticks_before = ksmd_cpu_ticks();
    let meminfo = fs::read_to_string("/proc/meminfo").map_err(|e| e.to_string())?;
    let available_kib: u64 = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .and_then(|tail| tail.split_whitespace().next())
        .ok_or("MemAvailable missing from /proc/meminfo")?
        .parse()
        .map_err(|e| format!("invalid MemAvailable: {e}"))?;
    let estimated_clone_budget_kib = (u64::from(count) * RAM_MIB + 512) * 1024;
    let required_available_kib = estimated_clone_budget_kib.max(2 * 1024 * 1024);
    if available_kib < required_available_kib {
        return Err(format!(
            "need at least {required_available_kib} KiB MemAvailable for {count} clones, found {available_kib} KiB"
        ));
    }
    fs::DirBuilder::new()
        .mode(0o700)
        .create(out)
        .map_err(|e| format!("create new output directory: {e}"))?;
    let out = out.canonicalize().map_err(|e| e.to_string())?;
    let mut guest = prepare_guest(&out, guest_dir)?;
    guest.merge = ksm;
    let base = out.join("base.qcow2");
    create_base(&base, &guest)?;

    let mut baseline_vms = Vec::new();
    let mut baseline_serials = Vec::new();
    let baseline_start = Instant::now();
    for index in 0..count {
        let name = format!("baseline-{index:03}");
        let disk = out.join(format!("{name}.qcow2"));
        checked(
            Command::new("qemu-img")
                .args(["convert", "-f", "qcow2", "-O", "qcow2"])
                .arg(&base)
                .arg(&disk),
        )?;
        let baseline_identity = CloneIdentity::generate()?;
        let mac = &baseline_identity.mac;
        let vm = launch(
            &out,
            &name,
            &disk,
            RamBackend::Anonymous,
            &guest,
            Some(&mac),
            false,
        )?;
        let mut serial = vm.serial()?;
        serial.expect("READY", Duration::from_secs(60))?;
        prepare_identity(&mut serial, &baseline_identity)?;
        baseline_serials.push(serial);
        baseline_vms.push(vm);
    }
    let baseline_elapsed_ms = baseline_start.elapsed().as_millis();
    let baseline_pids: Vec<_> = baseline_vms.iter().map(Vm::pid).collect();
    let (baseline_cpu_sample_interval_ms, baseline_cpu_ticks) = sample_cpu_ticks(&baseline_pids)?;
    let mut baseline_serial_refs: Vec<_> = baseline_serials.iter_mut().collect();
    let (baseline_active_cpu_sample_interval_ms, baseline_active_cpu_ticks) =
        sample_active_cpu_ticks(&baseline_pids, &mut baseline_serial_refs)?;
    let ticks_per_second = host["clock_ticks_per_second"].as_u64().unwrap_or(100);
    let mut baseline_measurements = Vec::new();
    for (index, vm) in baseline_vms.iter().enumerate() {
        let disk = out.join(format!("baseline-{index:03}.qcow2"));
        baseline_measurements.push(json!({"index":index,"pid":vm.pid(),"qemu_command":vm.command,"memory":measure(vm.pid())?,"idle_cpu_ticks":baseline_cpu_ticks[index],"active_cpu_ticks":baseline_active_cpu_ticks[index],"active_cpu_percent_one_core":cpu_ticks_as_percent_of_one_core(baseline_active_cpu_ticks[index], baseline_active_cpu_sample_interval_ms, ticks_per_second),"disk_allocated_bytes":allocated_bytes(&disk)?}));
    }
    drop(baseline_serial_refs);
    drop(baseline_serials);
    drop(baseline_vms);

    if range {
        let port = 30000
            + u32::from_str_radix(&clone_identities[0].machine_id[..4], 16)
                .map_err(|e| e.to_string())?
                % 30000;
        guest.netdev = Some(format!(
            "socket,id=net0,mcast=230.0.0.1:{port},localaddr=127.0.0.1"
        ));
    }

    let ram = out.join("memory.img");
    File::create(&ram)
        .and_then(|file| file.set_len(RAM_BYTES))
        .map_err(|e| e.to_string())?;
    let mut template = launch(
        &out,
        "template",
        &base,
        RamBackend::File {
            path: &ram,
            source: true,
        },
        &guest,
        None,
        false,
    )?;
    let mut template_serial = template.serial()?;
    template_serial.expect("READY", Duration::from_secs(60))?;
    if !expect_value(&mut template_serial, "ram-read", "RAM 90")?
        || !expect_value(&mut template_serial, "disk-read", "DISK 0")?
    {
        return Err("template guest initial values differ".into());
    }
    let mut template_qmp = template.qmp()?;
    template_serial.command("sync", "SYNCED")?;
    let distribution = if guest.disk.is_some() {
        let services = template_serial.command("services-read", "SERVICES ")?;
        let login = login_prompt(
            template
                .login_stream
                .as_mut()
                .ok_or("missing login console")?,
        )?;
        if services != "SERVICES init=true syslogd=true crond=true getty=true" || !login {
            return Err(format!(
                "distribution checkpoint is not ready: {services}; login_prompt={login}"
            ));
        }
        json!({"services":services,"serial_login_prompt":login,"disk_probe_scope":"64 MiB root filesystem test file"})
    } else {
        json!({"kind":"synthetic","disk_probe_scope":"entire 64 MiB raw test disk"})
    };
    if template_qmp.execute("query-kvm", None)?["enabled"] != true {
        return Err("template is not executing with KVM".into());
    }
    let template_command = template.command.clone();
    let template_vmgenid = template_qmp.execute("query-vm-generation-id", None)?["guid"]
        .as_str()
        .ok_or("template vmgenid missing")?
        .to_string();
    template_qmp.execute("stop", None)?;
    if template_qmp.execute("query-status", None)?["status"] != "paused" {
        return Err("template did not stop".into());
    }
    enable_ignore_shared(&mut template_qmp)?;
    let state = out.join("machine-state");
    template_qmp.execute(
        "migrate",
        Some(json!({"uri":format!("file:{}",state.display())})),
    )?;
    let template_migration = wait_migration(&mut template_qmp)?;
    template_qmp.execute("quit", None)?;
    let shutdown_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if template
            .child
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_some()
        {
            break;
        }
        if Instant::now() >= shutdown_deadline {
            return Err("template QEMU did not exit after QMP quit".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
    drop(template);
    for artifact in [&ram, &base, &state] {
        File::open(artifact)
            .and_then(|file| file.sync_all())
            .map_err(|e| format!("sync {}: {e}", artifact.display()))?;
    }
    fs::set_permissions(&ram, fs::Permissions::from_mode(0o444)).map_err(|e| e.to_string())?;
    fs::set_permissions(&base, fs::Permissions::from_mode(0o444)).map_err(|e| e.to_string())?;
    let ram_hash_before = sha256(&ram)?;

    let clone_start = Instant::now();
    let mut overlays = Vec::new();
    for index in 0..count {
        let name = format!("clone-{index:03}");
        let overlay = out.join(format!("{name}.qcow2"));
        checked(
            Command::new("qemu-img")
                .args(["create", "-f", "qcow2", "-b"])
                .arg(&base)
                .args(["-B", "qcow2"])
                .arg(&overlay),
        )?;
        verify_backing(&overlay, &base)?;
        overlays.push(overlay);
    }
    let mut a = launch(
        &out,
        "clone-000",
        &overlays[0],
        RamBackend::File {
            path: &ram,
            source: false,
        },
        &guest,
        None,
        true,
    )?;
    let b = launch(
        &out,
        "clone-001",
        &overlays[1],
        RamBackend::File {
            path: &ram,
            source: false,
        },
        &guest,
        None,
        true,
    )?;
    let mut aq = a.qmp()?;
    let mut bq = b.qmp()?;
    let a_migration = restore(&mut aq, &state)?;
    let b_migration = restore(&mut bq, &state)?;
    let a_vmgenid = aq.execute("query-vm-generation-id", None)?["guid"]
        .as_str()
        .ok_or("clone A vmgenid missing")?
        .to_string();
    let b_vmgenid = bq.execute("query-vm-generation-id", None)?["guid"]
        .as_str()
        .ok_or("clone B vmgenid missing")?
        .to_string();
    let mut vmgenid_unique =
        a_vmgenid != b_vmgenid && a_vmgenid != template_vmgenid && b_vmgenid != template_vmgenid;
    let mut extra_clones = Vec::new();
    for index in 2..count {
        let name = format!("clone-{index:03}");
        let vm = launch(
            &out,
            &name,
            &overlays[index as usize],
            RamBackend::File {
                path: &ram,
                source: false,
            },
            &guest,
            None,
            true,
        )?;
        let mut qmp = vm.qmp()?;
        let migration = restore(&mut qmp, &state)?;
        let identity = &clone_identities[index as usize];
        let mut serial = vm.serial()?;
        let identity_guest = prepare_identity(&mut serial, identity)?;
        add_nic(&mut qmp, &identity.mac)?;
        let mac_guest = serial.command("mac-read", "MAC ")?;
        let vmgenid = qmp.execute("query-vm-generation-id", None)?["guid"]
            .as_str()
            .ok_or("extra clone vmgenid missing")?
            .to_string();
        extra_clones.push((
            vm,
            migration,
            serial,
            index,
            identity_guest,
            mac_guest,
            vmgenid,
        ));
    }
    let mut aserial = a.serial()?;
    let mut bserial = b.serial()?;
    aserial.command("ping", "PONG")?;
    bserial.command("ping", "PONG")?;
    let a_identity = &clone_identities[0].machine_id;
    let b_identity = &clone_identities[1].machine_id;
    let a_identity_guest = prepare_identity(&mut aserial, &clone_identities[0])?;
    let b_identity_guest = prepare_identity(&mut bserial, &clone_identities[1])?;
    add_nic(&mut aq, &clone_identities[0].mac)?;
    add_nic(&mut bq, &clone_identities[1].mac)?;
    let mut guest_identities = vec![a_identity_guest.clone(), b_identity_guest.clone()];
    let mut identity_unique = identity_matches(&a_identity_guest, &a_identity)
        && identity_matches(&b_identity_guest, &b_identity)
        && a_identity_guest != b_identity_guest;
    let mut all_vmgenids = vec![
        a_vmgenid.clone(),
        b_vmgenid.clone(),
        template_vmgenid.clone(),
    ];
    for (_, _, _, index, identity_guest, _, vmgenid) in &extra_clones {
        let expected_identity = &clone_identities[*index as usize].machine_id;
        identity_unique &= identity_matches(identity_guest, &expected_identity)
            && !guest_identities.contains(identity_guest);
        guest_identities.push(identity_guest.clone());
        vmgenid_unique &= !all_vmgenids.iter().any(|existing| existing == vmgenid);
        all_vmgenids.push(vmgenid.clone());
    }
    let a_mac_guest = aserial.command("mac-read", "MAC ")?;
    let b_mac_guest = bserial.command("mac-read", "MAC ")?;
    let mut range_before_verified = !range;
    let mut range_network_benchmark = None;
    if range {
        aserial.command("network-set 10.77.0.10", "NETWORK ")?;
        bserial.command("network-set 10.77.0.11", "NETWORK ")?;
        for (_, _, serial, index, _, _, _) in &mut extra_clones {
            serial.command(&format!("network-set 10.77.0.{}", *index + 10), "NETWORK ")?;
        }
        let attacker = &mut extra_clones.last_mut().ok_or("missing attacker")?.2;
        range_before_verified = true;
        for index in 0..count - 1 {
            let reply = attacker.command(&format!("udp-probe 10.77.0.{}", index + 10), "UDP ")?;
            range_before_verified &=
                reply == format!("UDP {}", clone_identities[index as usize].machine_id);
        }
        let benchmark = attacker.command("udp-bench 10.77.0.10 64", "UDP-BENCH ")?;
        range_before_verified &= benchmark.contains(&clone_identities[0].machine_id);
        range_network_benchmark = Some(benchmark);
    }
    let clone_elapsed_ms = clone_start.elapsed().as_millis();
    let process_isolation = if guest.disk.is_some() {
        aserial.command("process-start", "PROCESS-STARTED")?;
        expect_value(&mut aserial, "process-count", "PROCESS-COUNT 1")?
            && expect_value(&mut bserial, "process-count", "PROCESS-COUNT 0")?
    } else {
        true
    };
    let mut extra_guest_isolation = true;
    for (_, _, serial, index, _, _, _) in &mut extra_clones {
        let ram_value = 100 + *index as u8;
        let disk_value = 10 + *index as u8;
        serial.command(
            &format!("ram-write-pages 1 {ram_value}"),
            &format!("RAM-WROTE-PAGES 1 {ram_value}"),
        )?;
        serial.command(
            &format!("disk-write {disk_value}"),
            &format!("DISK-WROTE {disk_value}"),
        )?;
    }
    let mut clone_pids = vec![a.pid(), b.pid()];
    clone_pids.extend(extra_clones.iter().map(|(vm, _, _, _, _, _, _)| vm.pid()));
    let (clone_cpu_sample_interval_ms, clone_idle_cpu_ticks) = sample_cpu_ticks(&clone_pids)?;
    let mut clone_serial_refs: Vec<_> = vec![&mut aserial, &mut bserial];
    for (_, _, serial, _, _, _, _) in &mut extra_clones {
        clone_serial_refs.push(serial);
    }
    let (clone_active_cpu_sample_interval_ms, clone_active_cpu_ticks) =
        sample_active_cpu_ticks(&clone_pids, &mut clone_serial_refs)?;
    drop(clone_serial_refs);
    for (_, _, serial, index, _, _, _) in &mut extra_clones {
        let ram_value = 100 + *index as u8;
        let disk_value = 10 + *index as u8;
        extra_guest_isolation &= serial.command("ram-read", "RAM ")? == format!("RAM {ram_value}")
            && serial.command("disk-read", "DISK ")? == format!("DISK {disk_value}");
    }
    extra_guest_isolation &= expect_value(&mut aserial, "ram-read", "RAM 90")?
        && expect_value(&mut bserial, "ram-read", "RAM 90")?
        && expect_value(&mut aserial, "disk-read", "DISK 0")?
        && expect_value(&mut bserial, "disk-read", "DISK 0")?;
    let initial_ok = expect_value(&mut aserial, "ram-read", "RAM 90")?
        && expect_value(&mut bserial, "ram-read", "RAM 90")?
        && expect_value(&mut aserial, "disk-read", "DISK 0")?
        && expect_value(&mut bserial, "disk-read", "DISK 0")?;
    let before_a = measure(a.pid())?;
    let before_b = measure(b.pid())?;
    let before_ram_a = measure_mapping(a.pid(), &ram)?;
    let before_ram_b = measure_mapping(b.pid(), &ram)?;
    let overlay_before = [
        allocated_bytes(&overlays[0])?,
        allocated_bytes(&overlays[1])?,
    ];
    let snapshot_before = snapshot(&clone_pids, &ram)?;
    aserial.command(
        &format!("ram-random-pages {divergence_pages} 165"),
        &format!("RAM-WROTE-PAGES {divergence_pages} 165"),
    )?;
    let a_expected_ram = if divergence_pages == 0 { 90 } else { 165 };
    let ram_isolation = initial_ok
        && expect_value(&mut aserial, "ram-read", &format!("RAM {a_expected_ram}"))?
        && expect_value(&mut bserial, "ram-read", "RAM 90")?;
    let after_ram_a = measure(a.pid())?;
    let after_ram_b = measure(b.pid())?;
    let after_ram_mapping_a = measure_mapping(a.pid(), &ram)?;
    let after_ram_mapping_b = measure_mapping(b.pid(), &ram)?;
    let memory_file_resident_bytes = resident_file_bytes(&ram)?;
    let mut divergence_all_verified = true;
    if divergence_all {
        bserial.command(
            &format!("ram-random-pages {divergence_pages} 166"),
            "RAM-WROTE-PAGES ",
        )?;
        for (_, _, serial, index, _, _, _) in &mut extra_clones {
            serial.command(
                &format!("ram-random-pages {divergence_pages} {}", 167 + *index),
                "RAM-WROTE-PAGES ",
            )?;
        }
        divergence_all_verified &=
            expect_value(&mut aserial, "ram-read", &format!("RAM {a_expected_ram}"))?;
        divergence_all_verified &= expect_value(
            &mut bserial,
            "ram-read",
            if divergence_pages == 0 {
                "RAM 90"
            } else {
                "RAM 166"
            },
        )?;
        for (_, _, serial, index, _, _, _) in &mut extra_clones {
            let value = if divergence_pages == 0 {
                100 + *index
            } else {
                167 + *index
            };
            divergence_all_verified &= expect_value(serial, "ram-read", &format!("RAM {value}"))?;
        }
    }
    let snapshot_after = snapshot(&clone_pids, &ram)?;
    aserial.command("disk-write 165", "DISK-WROTE 165")?;
    let disk_isolation = expect_value(&mut aserial, "disk-read", "DISK 165")?
        && expect_value(&mut bserial, "disk-read", "DISK 0")?;
    bserial.command("disk-write 66", "DISK-WROTE 66")?;
    let disk_isolation = disk_isolation
        && expect_value(&mut aserial, "disk-read", "DISK 165")?
        && expect_value(&mut bserial, "disk-read", "DISK 66")?;
    aserial.command("disk-fill 165", "DISK-FILLED 165")?;
    let full_disk_isolation = expect_value(&mut bserial, "disk-read", "DISK 66")?;
    let a_overlay_after_full_write = allocated_bytes(&overlays[0])?;
    let mut parking_verified = !park_idle;
    let mut parking_details = json!({"requested":park_idle});
    if park_idle {
        let mut paused_statuses = Vec::new();
        let mut resumed_statuses = Vec::new();
        paused_statuses.push(stop_and_verify(&mut aq)?);
        paused_statuses.push(stop_and_verify(&mut bq)?);
        let mut extra_qmps = Vec::new();
        for (vm, _, _, _, _, _, _) in &extra_clones {
            let mut qmp = vm.qmp()?;
            paused_statuses.push(stop_and_verify(&mut qmp)?);
            extra_qmps.push(qmp);
        }
        let (parked_cpu_sample_interval_ms, parked_cpu_ticks) = sample_cpu_ticks(&clone_pids)?;
        resumed_statuses.push(resume_and_verify(&mut aq)?);
        resumed_statuses.push(resume_and_verify(&mut bq)?);
        for qmp in &mut extra_qmps {
            resumed_statuses.push(resume_and_verify(qmp)?);
        }
        let mut guests_responsive_after_resume = expect_value(&mut aserial, "ping", "PONG")?
            && expect_value(&mut bserial, "ping", "PONG")?;
        for (_, _, serial, _, _, _, _) in &mut extra_clones {
            guests_responsive_after_resume &= expect_value(serial, "ping", "PONG")?;
        }
        parking_verified = paused_statuses.iter().all(|paused| *paused)
            && resumed_statuses.iter().all(|running| *running)
            && guests_responsive_after_resume;
        parking_details = json!({
            "requested":true,
            "paused_statuses":paused_statuses,
            "resumed_statuses":resumed_statuses,
            "cpu_sample_interval_ms":parked_cpu_sample_interval_ms,
            "qemu_process_cpu_ticks":parked_cpu_ticks,
            "guests_responsive_after_resume":guests_responsive_after_resume,
            "wall_clock_semantics":"guest timers stop while paused; explicit QMP cont resumes execution"
        });
    }
    // `-no-reboot` turns a guest reboot into QEMU shutdown. The guest may reset
    // its UART before the acknowledgement drains, so observe the process/QMP
    // state instead of waiting for a serial reply that can be lost on reset.
    aserial.send("reboot")?;
    let reboot_deadline = Instant::now() + Duration::from_secs(5);
    let mut a_reboot_process_status = None;
    let mut a_reboot_qmp_status = None;
    while Instant::now() < reboot_deadline {
        a_reboot_process_status = a.child.try_wait().map_err(|e| e.to_string())?;
        if a_reboot_process_status.is_some() {
            a_reboot_qmp_status = None;
            break;
        }
        a_reboot_qmp_status = aq
            .execute("query-status", None)
            .ok()
            .and_then(|status| status["status"].as_str().map(str::to_owned));
        if a_reboot_qmp_status
            .as_deref()
            .is_some_and(|status| status != "running")
        {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let a_reboot_observed = a_reboot_process_status.is_some()
        || a_reboot_qmp_status
            .as_deref()
            .is_some_and(|status| status != "running");
    let b_survives_guest_reboot = expect_value(&mut bserial, "ping", "PONG")?;
    let overlay_after = [
        allocated_bytes(&overlays[0])?,
        allocated_bytes(&overlays[1])?,
    ];
    let ram_hash_after = sha256(&ram)?;
    let template_ram_unchanged = ram_hash_before == ram_hash_after;
    let mac_unique = a_mac_guest == format!("MAC {}", clone_identities[0].mac)
        && b_mac_guest == format!("MAC {}", clone_identities[1].mac);
    a.child.kill().map_err(|e| e.to_string())?;
    a.child.wait().map_err(|e| e.to_string())?;
    let b_survives_a_kill = expect_value(&mut bserial, "ping", "PONG")?;
    let mut range_after_verified = !range;
    if range {
        let attacker = &mut extra_clones.last_mut().ok_or("missing attacker")?.2;
        range_after_verified = attacker.command("udp-probe 10.77.0.11", "UDP ")?
            == format!("UDP {}", clone_identities[1].machine_id);
    }
    let mut surviving_clones_verified = expect_value(&mut bserial, "disk-read", "DISK 66")?;
    surviving_clones_verified &= expect_value(
        &mut bserial,
        "ram-read",
        if divergence_all && divergence_pages > 0 {
            "RAM 166"
        } else {
            "RAM 90"
        },
    )?;
    for (_, _, serial, index, identity, _, _) in &mut extra_clones {
        let expected_ram = if divergence_all && divergence_pages > 0 {
            167 + *index
        } else {
            100 + *index
        };
        surviving_clones_verified &= expect_value(serial, "ping", "PONG")?
            && serial.command("identity-read", "IDENTITY ")? == *identity
            && serial.command("disk-read", "DISK ")? == format!("DISK {}", 10 + *index)
            && serial.command("ram-read", "RAM ")? == format!("RAM {expected_ram}");
    }
    let extra_measurements = extra_clones
        .iter()
        .map(
            |(vm, migration, _, index, identity_guest, mac_guest, vmgenid)| {
                Ok(json!({
                    "index": index,
                    "pid": vm.pid(),
                    "qemu_command": vm.command,
                    "memory": measure(vm.pid())?,
                    "ram_mapping": measure_mapping(vm.pid(), &ram)?,
                    "idle_cpu_ticks": clone_idle_cpu_ticks[*index as usize],
                    "active_cpu_ticks": clone_active_cpu_ticks[*index as usize],
                    "active_cpu_percent_one_core": cpu_ticks_as_percent_of_one_core(clone_active_cpu_ticks[*index as usize], clone_active_cpu_sample_interval_ms, ticks_per_second),
                    "overlay_allocated_bytes": allocated_bytes(&overlays[*index as usize])?,
                    "identity_guest": identity_guest,
                    "mac_guest": mac_guest,
                    "vmgenid": vmgenid,
                    "migration": migration,
                }))
            },
        )
        .collect::<Result<Vec<_>, String>>()?;
    let mut all_macs_unique = mac_unique;
    for (_, _, _, index, _, mac_guest, _) in &extra_clones {
        all_macs_unique &= mac_guest == &format!("MAC {}", clone_identities[*index as usize].mac);
    }
    let total_overlay_allocated_bytes = overlays
        .iter()
        .map(|overlay| allocated_bytes(overlay))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .sum::<u64>();
    let ksm_counters_after = crate::inspect::ksm_status().ok();
    let ksmd_cpu_ticks_after = ksmd_cpu_ticks();
    let ksmd_cpu_ticks_delta = ksmd_cpu_ticks_before
        .zip(ksmd_cpu_ticks_after)
        .and_then(|(before, after)| after.checked_sub(before));
    let mut result = json!({
        "experiment":"two-clone-cow-probe",
        "host":host,
        "guest":guest.metadata,
        "distribution_checkpoint":distribution,
        "qemu_version":output(Command::new("qemu-system-x86_64").arg("--version"))?.lines().next(),
        "kernel_version":output(Command::new("uname").arg("-r"))?.trim(),
        "guest_ram_mib":RAM_MIB,
        "divergence_region_bytes":u64::from(TEST_REGION_MIB)*1024*1024,
        "divergence_scope":if divergence_all { "all clones" } else { "clone A only" },
        "divergence_all_verified":divergence_all_verified,
        "snapshots":{"before_divergence":snapshot_before,"after_divergence":snapshot_after},
        "ram_divergence_pages":divergence_pages,
        "ram_divergence_bytes":u64::from(divergence_pages) * 4096,
        "effective_ram_divergence_percent":(u64::from(divergence_pages) * 100 + u64::from(TEST_REGION_PAGES) / 2) / u64::from(TEST_REGION_PAGES),
        "host_mem_available_kib_at_start":available_kib,
        "host_mem_required_kib":required_available_kib,
        "ksm_run":fs::read_to_string("/sys/kernel/mm/ksm/run").unwrap_or_default().trim(),
        "ksm_telemetry":{"scope":"host-global counters; other KSM workloads may affect deltas","counters_before":ksm_counters_before,"counters_after":ksm_counters_after,"ksmd_cpu_ticks_before":ksmd_cpu_ticks_before,"ksmd_cpu_ticks_after":ksmd_cpu_ticks_after,"ksmd_cpu_ticks_delta":ksmd_cpu_ticks_delta},
        "baseline":{"count":count,"vms":baseline_measurements,"time_to_count_ready_ms":baseline_elapsed_ms,"idle_cpu_sample_interval_ms":baseline_cpu_sample_interval_ms,"active_cpu_sample_interval_ms":baseline_active_cpu_sample_interval_ms},
        "template":{"qemu_command":template_command,"memory_file_bytes":RAM_BYTES,"memory_file_allocated_bytes":allocated_bytes(&ram)?,"memory_file_resident_bytes":memory_file_resident_bytes,"memory_sha256":ram_hash_after,"vmgenid":template_vmgenid,"machine_state_allocated_bytes":allocated_bytes(&state)?,"migration":template_migration},
        "disk_storage":{"base_allocated_bytes":allocated_bytes(&base)?,"total_clone_overlay_allocated_bytes":total_overlay_allocated_bytes},
        "clones":{"count":count,"idle_cpu_sample_interval_ms":clone_cpu_sample_interval_ms,"a":{"pid":a.pid(),"qemu_command":a.command,"memory":before_a,"ram_mapping":before_ram_a,"idle_cpu_ticks":clone_idle_cpu_ticks[0],"after_ram_write_memory":after_ram_a,"after_ram_write_ram_mapping":after_ram_mapping_a,"mac_guest":a_mac_guest,"identity_guest":a_identity_guest,"vmgenid":a_vmgenid,"overlay_initial_allocated_bytes":overlay_before[0],"overlay_final_allocated_bytes":overlay_after[0],"overlay_after_full_write_allocated_bytes":a_overlay_after_full_write,"migration":a_migration},
                  "b":{"pid":b.pid(),"qemu_command":b.command,"memory":before_b,"ram_mapping":before_ram_b,"idle_cpu_ticks":clone_idle_cpu_ticks[1],"after_a_ram_write_memory":after_ram_b,"after_a_ram_write_ram_mapping":after_ram_mapping_b,"mac_guest":b_mac_guest,"identity_guest":b_identity_guest,"vmgenid":b_vmgenid,"overlay_initial_allocated_bytes":overlay_before[1],"overlay_final_allocated_bytes":overlay_after[1],"migration":b_migration},
                  "extra":extra_measurements,
                  "time_to_count_resumed_ms":clone_elapsed_ms},
        "ram_isolation":ram_isolation,"disk_isolation":disk_isolation,"extra_clone_isolation":extra_guest_isolation,"full_disk_isolation":full_disk_isolation,"b_survives_guest_reboot":b_survives_guest_reboot,"template_ram_unchanged":template_ram_unchanged,"backing_verified":true,"mac_unique":all_macs_unique,"identity_unique":identity_unique,"vmgenid_unique":vmgenid_unique,"b_survives_a_kill":b_survives_a_kill,"parking_verified":parking_verified,"parking":parking_details
    });
    result["clones"]["active_cpu_sample_interval_ms"] = json!(clone_active_cpu_sample_interval_ms);
    result["clones"]["a"]["active_cpu_ticks"] = json!(clone_active_cpu_ticks[0]);
    result["clones"]["a"]["active_cpu_percent_one_core"] = json!(cpu_ticks_as_percent_of_one_core(
        clone_active_cpu_ticks[0],
        clone_active_cpu_sample_interval_ms,
        ticks_per_second
    ));
    result["clones"]["b"]["active_cpu_ticks"] = json!(clone_active_cpu_ticks[1]);
    result["clones"]["b"]["active_cpu_percent_one_core"] = json!(cpu_ticks_as_percent_of_one_core(
        clone_active_cpu_ticks[1],
        clone_active_cpu_sample_interval_ms,
        ticks_per_second
    ));
    result["active_cpu_workload"] = json!({"guest_command":format!("cpu-burn {ACTIVE_CPU_BURN_MS}"),"sample_interval_ms":clone_active_cpu_sample_interval_ms,"cpu_ticks_units":"raw /proc/<pid>/stat scheduler ticks","ticks_per_second":ticks_per_second});
    result["range"] = json!({"enabled":range,"backend":guest.netdev,"attacker_index":if range {Some(count-1)} else {None},"target_count":if range {Some(count-1)} else {None},"connectivity_before":range_before_verified,"connectivity_after_target_kill":range_after_verified,"network_benchmark":range_network_benchmark});
    result["process_isolation"] = json!(process_isolation);
    result["surviving_clones_verified"] = json!(surviving_clones_verified);
    result["a_guest_reboot_observed"] = json!(a_reboot_observed);
    result["a_reboot_process_exited"] = json!(a_reboot_process_status.is_some());
    result["a_reboot_qmp_status"] = json!(a_reboot_qmp_status);
    fs::write(
        out.join("result.json"),
        serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    println!("{}", out.join("result.json").display());
    if !(ram_isolation
        && disk_isolation
        && template_ram_unchanged
        && all_macs_unique
        && identity_unique
        && vmgenid_unique
        && extra_guest_isolation
        && full_disk_isolation
        && b_survives_guest_reboot
        && b_survives_a_kill
        && parking_verified
        && divergence_all_verified
        && process_isolation
        && surviving_clones_verified
        && a_reboot_observed
        && range_before_verified
        && range_after_verified)
    {
        return Err("one or more isolation checks failed; see result.json".into());
    }
    Ok(())
}

fn prepare_identity(serial: &mut Serial, identity: &CloneIdentity) -> Result<String, String> {
    serial.command(
        &format!("entropy-add {}", identity.entropy_hex()),
        "RNG-RESEEDED",
    )?;
    serial.command(
        &format!("identity-set {}", identity.machine_id),
        "IDENTITY-SET ",
    )?;
    serial.command("identity-read", "IDENTITY ")
}

fn identity_matches(reply: &str, identity: &str) -> bool {
    reply
        == format!(
            "IDENTITY {identity} densevm-{} RNG-RESEEDED",
            &identity[..12]
        )
}

pub fn run_baseline(out: &Path, count: u32) -> Result<(), String> {
    if count == 0 || count > 32 {
        return Err("baseline count must be between 1 and 32".to_string());
    }
    let host = crate::capabilities::host_report();
    let available_kib: u64 = fs::read_to_string("/proc/meminfo")
        .map_err(|e| e.to_string())?
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .and_then(|tail| tail.split_whitespace().next())
        .ok_or("MemAvailable missing from /proc/meminfo")?
        .parse()
        .map_err(|e| format!("invalid MemAvailable: {e}"))?;
    let required_kib = u64::from(count) * 300 * 1024;
    if available_kib < required_kib {
        return Err(format!(
            "need {required_kib} KiB available for {count} baseline VMs; found {available_kib} KiB"
        ));
    }
    fs::create_dir(out).map_err(|e| format!("create new output directory: {e}"))?;
    let out = out.canonicalize().map_err(|e| e.to_string())?;
    let guest = prepare_guest(&out, None)?;
    let base = out.join("base.qcow2");
    checked(
        Command::new("qemu-img")
            .args(["create", "-f", "qcow2"])
            .arg(&base)
            .arg("64M"),
    )?;
    let start = Instant::now();
    let mut vms = Vec::new();
    let mut serials = Vec::new();
    for index in 0..count {
        let name = format!("baseline-{index:03}");
        let disk = out.join(format!("{name}.qcow2"));
        checked(
            Command::new("qemu-img")
                .args(["convert", "-f", "qcow2", "-O", "qcow2"])
                .arg(&base)
                .arg(&disk),
        )?;
        let mac = CloneIdentity::generate()?.mac;
        let vm = launch(
            &out,
            &name,
            &disk,
            RamBackend::Anonymous,
            &guest,
            Some(&mac),
            false,
        )?;
        let mut serial = vm.serial()?;
        serial.expect("READY", Duration::from_secs(60))?;
        serials.push(serial);
        vms.push(vm);
    }
    let elapsed = start.elapsed().as_millis();
    let pids: Vec<_> = vms.iter().map(Vm::pid).collect();
    let (idle_cpu_sample_interval_ms, idle_cpu_ticks) = sample_cpu_ticks(&pids)?;
    let mut serial_refs: Vec<_> = serials.iter_mut().collect();
    let (active_cpu_sample_interval_ms, active_cpu_ticks) =
        sample_active_cpu_ticks(&pids, &mut serial_refs)?;
    let ticks_per_second = host["clock_ticks_per_second"].as_u64().unwrap_or(100);
    let mut guests = Vec::new();
    for index in 0..count {
        let vm = &vms[index as usize];
        let disk = out.join(format!("baseline-{index:03}.qcow2"));
        guests.push(json!({"index":index,"pid":vm.pid(),"qemu_command":vm.command,"memory":measure(vm.pid())?,"idle_cpu_ticks":idle_cpu_ticks[index as usize],"active_cpu_ticks":active_cpu_ticks[index as usize],"active_cpu_percent_one_core":cpu_ticks_as_percent_of_one_core(active_cpu_ticks[index as usize], active_cpu_sample_interval_ms, ticks_per_second),"disk_allocated_bytes":allocated_bytes(&disk)?}));
    }
    let result = json!({"experiment":"baseline","host":host,"count":count,"guest_ram_mib":RAM_MIB,"host_mem_available_kib_at_start":available_kib,"time_to_ready_ms":elapsed,"idle_cpu_sample_interval_ms":idle_cpu_sample_interval_ms,"active_cpu_sample_interval_ms":active_cpu_sample_interval_ms,"active_cpu_workload":{"guest_command":format!("cpu-burn {ACTIVE_CPU_BURN_MS}"),"ticks_per_second":ticks_per_second},"cpu_ticks_units":"raw /proc/<pid>/stat scheduler ticks","vms":guests});
    fs::write(
        out.join("baseline.json"),
        serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    println!("{}", out.join("baseline.json").display());
    Ok(())
}
