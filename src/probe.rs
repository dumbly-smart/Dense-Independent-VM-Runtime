use crate::metrics::{MemoryTotals, parse_smaps_for_path, parse_smaps_rollup};
use crate::qmp::Qmp;
use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const RAM_MIB: u64 = 256;
const RAM_BYTES: u64 = RAM_MIB * 1024 * 1024;
const A_MAC: &str = "52:54:00:d5:00:02";
const B_MAC: &str = "52:54:00:d5:00:03";

struct Vm {
    child: Child,
    qmp_path: PathBuf,
    serial_path: PathBuf,
    command: Vec<String>,
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
        writeln!(self.writer, "{command}").map_err(|e| format!("serial write: {e}"))?;
        self.writer.flush().map_err(|e| e.to_string())?;
        self.expect(wanted, Duration::from_secs(12))
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

fn launch(
    out: &Path,
    name: &str,
    disk: &Path,
    backend: RamBackend<'_>,
    initramfs: &Path,
    mac_at_boot: Option<&str>,
    incoming: bool,
) -> Result<Vm, String> {
    let qmp_path = out.join(format!("{name}.qmp.sock"));
    let serial_path = out.join(format!("{name}.serial.sock"));
    let log = File::create(out.join(format!("{name}.qemu.log"))).map_err(|e| e.to_string())?;
    let mut cmd = Command::new("qemu-system-x86_64");
    cmd.args([
        "-machine",
        "pc-q35-11.1,accel=kvm",
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
        cmd.args(["-object", &opts, "-machine", "memory-backend=ram"]);
    }
    cmd.arg("-kernel")
        .arg("/boot/vmlinuz-linux")
        .arg("-initrd")
        .arg(initramfs)
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
    cmd.args([
        "-device",
        "virtio-blk-pci,drive=disk,id=blk0",
        "-netdev",
        "user,id=net0,restrict=on",
    ]);
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
    Ok(Vm {
        child,
        qmp_path,
        serial_path,
        command,
    })
}

fn measure(pid: u32) -> Result<MemoryTotals, String> {
    let text =
        fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).map_err(|e| e.to_string())?;
    parse_smaps_rollup(&text)
}

fn measure_mapping(pid: u32, path: &Path) -> Result<MemoryTotals, String> {
    let text = fs::read_to_string(format!("/proc/{pid}/smaps")).map_err(|e| e.to_string())?;
    parse_smaps_for_path(&text, path.to_str().ok_or("non-UTF8 RAM path")?)
}

fn allocated_bytes(path: &Path) -> Result<u64, String> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path)
        .map(|x| x.blocks() * 512)
        .map_err(|e| e.to_string())
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
    run_with_options_and_divergence(out, count, ksm, 32)
}

pub fn run_with_options_and_divergence(
    out: &Path,
    count: u32,
    ksm: bool,
    divergence_mib: u32,
) -> Result<(), String> {
    if !(2..=32).contains(&count) {
        return Err(
            "the template restore probe supports clone counts from 2 through 32".to_string(),
        );
    }
    let _ksm_guard = if ksm {
        Some(crate::ksm::KsmGuard::enable()?)
    } else {
        None
    };
    run_two_clone(out, count, divergence_mib)
}

fn run_two_clone(out: &Path, count: u32, divergence_mib: u32) -> Result<(), String> {
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
    fs::create_dir(out).map_err(|e| format!("create new output directory: {e}"))?;
    let out = out.canonicalize().map_err(|e| e.to_string())?;
    let initramfs = build_initramfs(&out)?;
    let base = out.join("base.qcow2");
    checked(
        Command::new("qemu-img")
            .args(["create", "-f", "qcow2"])
            .arg(&base)
            .arg("64M"),
    )?;

    let mut baseline_vms = Vec::new();
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
        let mac = mac_for(index);
        let vm = launch(
            &out,
            &name,
            &disk,
            RamBackend::Anonymous,
            &initramfs,
            Some(&mac),
            false,
        )?;
        let mut serial = vm.serial()?;
        serial.expect("READY", Duration::from_secs(60))?;
        baseline_vms.push(vm);
    }
    let baseline_elapsed_ms = baseline_start.elapsed().as_millis();
    let mut baseline_measurements = Vec::new();
    for (index, vm) in baseline_vms.iter().enumerate() {
        let disk = out.join(format!("baseline-{index:03}.qcow2"));
        baseline_measurements.push(json!({"index":index,"pid":vm.pid(),"qemu_command":vm.command,"memory":measure(vm.pid())?,"disk_allocated_bytes":allocated_bytes(&disk)?}));
    }
    drop(baseline_vms);

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
        &initramfs,
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
    let clone_start = Instant::now();
    let mut a = launch(
        &out,
        "clone-000",
        &overlays[0],
        RamBackend::File {
            path: &ram,
            source: false,
        },
        &initramfs,
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
        &initramfs,
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
    add_nic(&mut aq, A_MAC)?;
    add_nic(&mut bq, B_MAC)?;
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
            &initramfs,
            None,
            true,
        )?;
        let mut qmp = vm.qmp()?;
        let migration = restore(&mut qmp, &state)?;
        let mac = mac_for(index);
        add_nic(&mut qmp, &mac)?;
        extra_clones.push((vm, qmp, migration));
    }
    let clone_elapsed_ms = clone_start.elapsed().as_millis();
    let mut aserial = a.serial()?;
    let mut bserial = b.serial()?;
    aserial.command("ping", "PONG")?;
    bserial.command("ping", "PONG")?;
    let a_identity = identity_for(0);
    let b_identity = identity_for(1);
    aserial.command(&format!("identity-set {a_identity}"), "IDENTITY-SET ")?;
    bserial.command(&format!("identity-set {b_identity}"), "IDENTITY-SET ")?;
    let a_identity_guest = aserial.command("identity-read", "IDENTITY ")?;
    let b_identity_guest = bserial.command("identity-read", "IDENTITY ")?;
    let mut guest_identities = vec![a_identity_guest.clone(), b_identity_guest.clone()];
    let mut identity_unique = identity_matches(&a_identity_guest, &a_identity)
        && identity_matches(&b_identity_guest, &b_identity)
        && a_identity_guest != b_identity_guest;
    let a_mac_guest = aserial.command("mac-read", "MAC ")?;
    let b_mac_guest = bserial.command("mac-read", "MAC ")?;
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
    aserial.command(
        &format!("ram-write-mib {divergence_mib} 165"),
        &format!("RAM-WROTE-MIB {divergence_mib} 165"),
    )?;
    let ram_isolation = initial_ok
        && expect_value(&mut aserial, "ram-read", "RAM 165")?
        && expect_value(&mut bserial, "ram-read", "RAM 90")?;
    let after_ram_a = measure(a.pid())?;
    let after_ram_b = measure(b.pid())?;
    let after_ram_mapping_a = measure_mapping(a.pid(), &ram)?;
    let after_ram_mapping_b = measure_mapping(b.pid(), &ram)?;
    let memory_file_resident_bytes = resident_file_bytes(&ram)?;
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
    aserial.command("reboot", "").ok();
    thread::sleep(Duration::from_millis(500));
    let b_survives_guest_reboot = expect_value(&mut bserial, "ping", "PONG")?;
    let overlay_after = [
        allocated_bytes(&overlays[0])?,
        allocated_bytes(&overlays[1])?,
    ];
    let ram_hash_after = sha256(&ram)?;
    let template_ram_unchanged = ram_hash_before == ram_hash_after;
    let mac_unique = a_mac_guest == format!("MAC {A_MAC}") && b_mac_guest == format!("MAC {B_MAC}");
    a.child.kill().map_err(|e| e.to_string())?;
    a.child.wait().map_err(|e| e.to_string())?;
    let b_survives_a_kill = expect_value(&mut bserial, "ping", "PONG")?;
    let extra_measurements = extra_clones
        .iter_mut()
        .enumerate()
        .map(|(offset, (vm, qmp, migration))| {
            let index = offset as u32 + 2;
            let identity = identity_for(index);
            let mut serial = vm.serial()?;
            serial.command(&format!("identity-set {identity}"), "IDENTITY-SET ")?;
            let identity_guest = serial.command("identity-read", "IDENTITY ")?;
            let mac_guest = serial.command("mac-read", "MAC ")?;
            let vmgenid = qmp.execute("query-vm-generation-id", None)?["guid"]
                .as_str()
                .ok_or("extra clone vmgenid missing")?
                .to_string();
            identity_unique &= identity_matches(&identity_guest, &identity)
                && !guest_identities.contains(&identity_guest);
            guest_identities.push(identity_guest.clone());
            Ok(json!({
                "index": index,
                "pid": vm.pid(),
                "qemu_command": vm.command,
                "memory": measure(vm.pid())?,
                "ram_mapping": measure_mapping(vm.pid(), &ram)?,
                "identity_guest": identity_guest,
                "mac_guest": mac_guest,
                "vmgenid": vmgenid,
                "migration": migration,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut all_vmgenids = vec![
        a_vmgenid.clone(),
        b_vmgenid.clone(),
        template_vmgenid.clone(),
    ];
    for extra in &extra_measurements {
        let guid = extra["vmgenid"]
            .as_str()
            .ok_or("extra clone vmgenid missing")?;
        vmgenid_unique &= !all_vmgenids.iter().any(|existing| existing == guid);
        all_vmgenids.push(guid.to_string());
    }
    let mut all_macs_unique = mac_unique;
    for extra in &extra_measurements {
        let index = extra["index"].as_u64().ok_or("extra clone index missing")? as u32;
        all_macs_unique &= extra["mac_guest"] == format!("MAC {}", mac_for(index));
    }
    let result = json!({
        "experiment":"two-clone-cow-probe",
        "qemu_version":output(Command::new("qemu-system-x86_64").arg("--version"))?.lines().next(),
        "kernel_version":output(Command::new("uname").arg("-r"))?.trim(),
        "guest_ram_mib":RAM_MIB,
        "requested_ram_divergence_mib":divergence_mib,
        "host_mem_available_kib_at_start":available_kib,
        "host_mem_required_kib":required_available_kib,
        "ksm_run":fs::read_to_string("/sys/kernel/mm/ksm/run").unwrap_or_default().trim(),
        "baseline":{"count":count,"vms":baseline_measurements,"time_to_count_ready_ms":baseline_elapsed_ms},
        "template":{"qemu_command":template_command,"memory_file_bytes":RAM_BYTES,"memory_file_allocated_bytes":allocated_bytes(&ram)?,"memory_file_resident_bytes":memory_file_resident_bytes,"memory_sha256":ram_hash_after,"vmgenid":template_vmgenid,"machine_state_allocated_bytes":allocated_bytes(&state)?,"migration":template_migration},
        "clones":{"count":count,"a":{"pid":a.pid(),"qemu_command":a.command,"memory":before_a,"ram_mapping":before_ram_a,"after_ram_write_memory":after_ram_a,"after_ram_write_ram_mapping":after_ram_mapping_a,"mac_guest":a_mac_guest,"identity_guest":a_identity_guest,"vmgenid":a_vmgenid,"overlay_initial_allocated_bytes":overlay_before[0],"overlay_final_allocated_bytes":overlay_after[0],"overlay_after_full_write_allocated_bytes":a_overlay_after_full_write,"migration":a_migration},
                  "b":{"pid":b.pid(),"qemu_command":b.command,"memory":before_b,"ram_mapping":before_ram_b,"after_a_ram_write_memory":after_ram_b,"after_a_ram_write_ram_mapping":after_ram_mapping_b,"mac_guest":b_mac_guest,"identity_guest":b_identity_guest,"vmgenid":b_vmgenid,"overlay_initial_allocated_bytes":overlay_before[1],"overlay_final_allocated_bytes":overlay_after[1],"migration":b_migration},
                  "extra":extra_measurements,
                  "time_to_two_resumed_ms":clone_elapsed_ms},
        "ram_isolation":ram_isolation,"disk_isolation":disk_isolation,"full_disk_isolation":full_disk_isolation,"b_survives_guest_reboot":b_survives_guest_reboot,"template_ram_unchanged":template_ram_unchanged,"backing_verified":true,"mac_unique":all_macs_unique,"identity_unique":identity_unique,"vmgenid_unique":vmgenid_unique,"b_survives_a_kill":b_survives_a_kill
    });
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
        && full_disk_isolation
        && b_survives_guest_reboot
        && b_survives_a_kill)
    {
        return Err("one or more isolation checks failed; see result.json".into());
    }
    Ok(())
}

fn mac_for(index: u32) -> String {
    format!(
        "52:54:00:d5:{:02x}:{:02x}",
        (index / 256) & 0xff,
        index & 0xff
    )
}

fn identity_for(index: u32) -> String {
    format!("{:032x}", u64::from(index) + 1)
}

fn identity_matches(reply: &str, identity: &str) -> bool {
    reply == format!("IDENTITY {identity} densevm-{} {identity}", &identity[..8])
}

pub fn run_baseline(out: &Path, count: u32) -> Result<(), String> {
    if count == 0 || count > 32 {
        return Err("baseline count must be between 1 and 32".to_string());
    }
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
    let initramfs = build_initramfs(&out)?;
    let base = out.join("base.qcow2");
    checked(
        Command::new("qemu-img")
            .args(["create", "-f", "qcow2"])
            .arg(&base)
            .arg("64M"),
    )?;
    let start = Instant::now();
    let mut vms = Vec::new();
    for index in 0..count {
        let name = format!("baseline-{index:03}");
        let disk = out.join(format!("{name}.qcow2"));
        checked(
            Command::new("qemu-img")
                .args(["convert", "-f", "qcow2", "-O", "qcow2"])
                .arg(&base)
                .arg(&disk),
        )?;
        let mac = mac_for(index);
        let vm = launch(
            &out,
            &name,
            &disk,
            RamBackend::Anonymous,
            &initramfs,
            Some(&mac),
            false,
        )?;
        let mut serial = vm.serial()?;
        serial.expect("READY", Duration::from_secs(60))?;
        vms.push(vm);
    }
    let elapsed = start.elapsed().as_millis();
    let mut guests = Vec::new();
    for index in 0..count {
        let vm = &vms[index as usize];
        let disk = out.join(format!("baseline-{index:03}.qcow2"));
        guests.push(json!({"index":index,"pid":vm.pid(),"qemu_command":vm.command,"memory":measure(vm.pid())?,"disk_allocated_bytes":allocated_bytes(&disk)?}));
    }
    let result = json!({"experiment":"baseline","count":count,"guest_ram_mib":RAM_MIB,"host_mem_available_kib_at_start":available_kib,"time_to_ready_ms":elapsed,"vms":guests});
    fs::write(
        out.join("baseline.json"),
        serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    println!("{}", out.join("baseline.json").display());
    Ok(())
}
