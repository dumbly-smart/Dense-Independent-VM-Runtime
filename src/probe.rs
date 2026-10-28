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
    fn observe_raw(&mut self, wanted: &str, timeout: Duration) -> Result<bool, String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            crate::interrupt::check()?;
            let mut line = String::new();
            match self.reader.read_line(&mut line) {
                Ok(0) => thread::sleep(Duration::from_millis(20)),
                Ok(_) if line.contains(wanted) => return Ok(true),
                Ok(_) => {}
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => return Err(format!("serial read: {error}")),
            }
        }
        Ok(false)
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
    use std::os::unix::net::UnixListener;
    use std::time::SystemTime;

    fn scripted_qmp(script: Vec<(&'static str, Value)>) -> (Qmp, thread::JoinHandle<()>, PathBuf) {
        let stamp = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "densevm-qmp-test-{}-{stamp}.sock",
            std::process::id()
        ));
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(b"{\"QMP\":{\"version\":{}}}\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let handshake: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(handshake["execute"], "qmp_capabilities");
            let reply = json!({"return":{},"id":handshake["id"]});
            stream.write_all(format!("{reply}\n").as_bytes()).unwrap();
            for (command, mut reply) in script {
                line.clear();
                reader.read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(request["execute"], command);
                reply["id"] = request["id"].clone();
                stream.write_all(format!("{reply}\n").as_bytes()).unwrap();
            }
        });
        let qmp = Qmp::connect(&path, Duration::from_secs(2)).unwrap();
        (qmp, server, path)
    }

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

    #[test]
    fn summarizes_translation_events_within_a_log_phase() {
        let trace = "translate_block pc:0x10\ntranslate_block pc:0x10\n";
        let phase = summarize_translation_phase(trace, 0, trace.len() as u64).unwrap();
        assert_eq!(phase["status"], "recorded");
        assert_eq!(phase["translation_block_events"], 2);
        assert_eq!(phase["guest_pcs_seen_multiple_times"], 1);
    }

    #[test]
    fn reports_translation_phases_without_events() {
        let phase = summarize_translation_phase("", 0, 0).unwrap();
        assert_eq!(phase["status"], "no_translation_events");
        assert_eq!(phase["log_bytes"], 0);
    }

    #[test]
    fn rejects_translation_phase_boundaries_outside_the_log() {
        assert!(summarize_translation_phase("trace", 0, 6).is_err());
        assert!(summarize_translation_phase("trace", 4, 2).is_err());
    }

    #[test]
    fn parses_x86_general_purpose_registers_from_qemu_monitor_output() {
        let registers = parse_qemu_gprs(
            "CPU#0\nRAX=00000000000000aa RBX=00000000000000bb RCX=00000000000000cc\nR8 = 0x8 R9=0000000000000009\nRIP=ffffffff81000000 RFLAGS=00000246",
        )
        .unwrap();
        assert_eq!(registers["RAX"], 0xaa);
        assert_eq!(registers["RBX"], 0xbb);
        assert_eq!(registers["RCX"], 0xcc);
        assert_eq!(registers["R8"], 0x8);
        assert_eq!(registers["R9"], 0x9);
        assert!(!registers.contains_key("RIP"));
        assert!(parse_qemu_gprs("no register dump").is_err());
    }

    #[test]
    fn validates_live_vcpu_topology_from_qmp() {
        let cpus = json!([
            {"cpu-index": 0, "thread-id": 101},
            {"cpu-index": 1, "thread-id": 102}
        ]);
        assert_eq!(parse_qemu_vcpu_indexes(&cpus, 2).unwrap(), vec![0, 1]);
        assert!(parse_qemu_vcpu_indexes(&cpus, 1).is_err());
        assert!(parse_qemu_vcpu_indexes(&json!([{"cpu-index": 0}, {"cpu-index": 0}]), 2).is_err());
        assert!(parse_qemu_vcpu_indexes(&json!([{"thread-id": 101}]), 1).is_err());
        assert!(parse_qemu_vcpu_indexes(&json!({"cpu-index": 0}), 1).is_err());
    }

    #[test]
    fn parses_privilege_level_from_qemu_register_output() {
        assert_eq!(parse_qemu_cpl("RIP=1234 RFL=2 CPL=3 II=0").unwrap(), 3);
        assert_eq!(parse_qemu_cpl("RIP=ffff RFL=2 CPL=0 II=0").unwrap(), 0);
        assert!(parse_qemu_cpl("no CPL field").is_err());
    }

    #[test]
    fn parses_qemu_segment_base_from_info_registers() {
        let output = "CPU#0\nGS =0000 0000000000012345 ffffffff 00c00100\n";
        assert_eq!(parse_qemu_segment_base(output, "GS").unwrap(), 0x12345);
        assert!(parse_qemu_segment_base(output, "FS").is_err());
    }

    #[test]
    fn parses_rtc_alarm_interrupt_reply() {
        assert_eq!(
            parse_rtc_alarm_reply("RTC-ALARM-FIRED events=1 flags=160").unwrap(),
            (1, 160)
        );
        assert!(parse_rtc_alarm_reply("RTC-ALARM-FIRED events=no flags=32").is_err());
        assert!(parse_rtc_alarm_reply("RTC-ALARM-TIMEOUT").is_err());
    }

    #[test]
    fn qemu_register_snapshot_preserves_the_full_monitor_text() {
        let monitor_text = "CPU#0\nRAX=00000000000000aa CR0=0000000080010033\n";
        let (mut qmp, server, path) = scripted_qmp(vec![(
            "human-monitor-command",
            json!({"return":monitor_text}),
        )]);
        let (registers, captured_text) = qemu_register_snapshot(&mut qmp).unwrap();
        assert_eq!(registers["RAX"], 0xaa);
        assert_eq!(captured_text, monitor_text);
        server.join().unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn qemu_lapic_snapshot_preserves_monitor_text() {
        let monitor_text = "APIC ID 0\nTPR 0x0\nLVT Timer: masked\n";
        let (mut qmp, server, path) = scripted_qmp(vec![(
            "human-monitor-command",
            json!({"return":monitor_text}),
        )]);
        assert_eq!(qemu_lapic_snapshot(&mut qmp).unwrap(), monitor_text);
        server.join().unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn parses_task_priority_from_qemu_lapic_output() {
        assert_eq!(parse_qemu_lapic_tpr("APR 0x00 TPR 0x20 DFR 0x0f"), Ok(0x20));
        assert!(parse_qemu_lapic_tpr("APIC ID 0\nISR (none)").is_err());
        assert!(parse_qemu_lapic_tpr("TPR 0xnothex").is_err());
    }

    #[test]
    fn selects_distinct_low_priority_task_priorities() {
        let (a, b) = choose_lapic_tpr_targets(0x10, 0x10).unwrap();
        assert_eq!((a, b), (0x11, 0x12));
        assert_eq!(choose_lapic_tpr_targets(0x20, 0x30).unwrap(), (0x21, 0x31));
    }

    #[test]
    fn parses_disk_io_benchmark_fields_as_numbers() {
        let result = parse_disk_benchmark(
            "DISK-BENCH iterations=16 block_bytes=4096 mean_us=120 p50_us=100 p95_us=301",
        )
        .unwrap();
        assert_eq!(result["iterations"], 16);
        assert_eq!(result["block_bytes"], 4096);
        assert_eq!(result["mean_us"], 120);
        assert_eq!(result["p95_us"], 301);
        assert!(parse_disk_benchmark("DISK-BENCH iterations=bad").is_err());
    }

    #[test]
    fn rejects_qcow2_overlay_with_stale_backing_path() {
        let overlay = Path::new("/tmp/densevm/clone-000.qcow2");
        let base = Path::new("/tmp/densevm/base.qcow2");
        let chain = json!([
            {
                "filename":overlay.display().to_string(),
                "format":"qcow2",
                "full-backing-filename":"/tmp/old-template/base.qcow2",
                "backing-filename-format":"qcow2"
            },
            {"filename":base.display().to_string(),"format":"qcow2"}
        ]);
        assert!(verify_backing_chain(&chain, overlay, base).is_err());
    }

    #[test]
    fn accepts_only_the_requested_overlay_and_base_chain() {
        let overlay = Path::new("/tmp/densevm/clone-000.qcow2");
        let base = Path::new("/tmp/densevm/base.qcow2");
        let chain = json!([
            {
                "filename":overlay.display().to_string(),
                "format":"qcow2",
                "full-backing-filename":base.display().to_string(),
                "backing-filename-format":"qcow2"
            },
            {"filename":base.display().to_string(),"format":"qcow2"}
        ]);
        assert!(verify_backing_chain(&chain, overlay, base).is_ok());
        assert!(
            verify_backing_chain(&chain, Path::new("/tmp/densevm/clone-001.qcow2"), base).is_err()
        );
    }

    #[test]
    fn parking_fails_immediately_when_stop_does_not_pause_the_guest() {
        let (mut qmp, server, path) = scripted_qmp(vec![
            ("stop", json!({"return":{}})),
            ("query-status", json!({"return":{"status":"running"}})),
        ]);
        let error = stop_and_verify(&mut qmp).unwrap_err();
        assert!(error.contains("running"));
        server.join().unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn resume_fails_immediately_when_the_guest_remains_paused() {
        let (mut qmp, server, path) = scripted_qmp(vec![
            ("cont", json!({"return":{}})),
            ("query-status", json!({"return":{"status":"paused"}})),
        ]);
        let error = resume_and_verify(&mut qmp).unwrap_err();
        assert!(error.contains("paused"));
        server.join().unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn dropping_one_vm_reaps_only_its_owned_process() {
        let owned = Command::new("sleep").arg("30").spawn().unwrap();
        let owned_pid = owned.id();
        let mut sibling = Command::new("sleep").arg("30").spawn().unwrap();
        let sibling_pid = sibling.id();
        let vm = Vm {
            child: owned,
            qmp_path: PathBuf::new(),
            serial_path: PathBuf::new(),
            command: Vec::new(),
            login_stream: None,
        };

        drop(vm);
        let owned_reaped = !Path::new(&format!("/proc/{owned_pid}")).exists();
        let sibling_survived = Path::new(&format!("/proc/{sibling_pid}")).exists();
        sibling.kill().unwrap();
        sibling.wait().unwrap();
        assert!(owned_reaped, "dropping the VM did not reap its child");
        assert!(
            sibling_survived,
            "dropping the VM affected an unrelated child"
        );
    }

    #[test]
    fn parses_guest_oom_kill_counter_delta_without_confusing_other_termination() {
        assert_eq!(
            parse_oom_kill_delta(
                "PRESSURE-OOM workers_killed=1 oom_kill_delta=1 allocation_failures=0 spawn_failures=0"
            ),
            Ok(1)
        );
        assert_eq!(
            parse_oom_kill_delta(
                "PRESSURE-NO-OOM oom_kill_delta=0 allocation_failures=8 spawn_failures=0"
            ),
            Ok(0)
        );
        assert!(parse_oom_kill_delta("PRESSURE-TIMEOUT").is_err());
    }

    #[test]
    fn parses_guest_wall_and_monotonic_clock_samples() {
        assert_eq!(
            parse_guest_clock("CLOCK realtime_ms=1792843200123 uptime_ms=481234"),
            Ok((1_792_843_200_123, 481_234))
        );
        assert!(parse_guest_clock("CLOCK realtime_ms=1").is_err());
    }

    #[test]
    fn parses_guest_timerfd_expiration_samples() {
        assert_eq!(
            parse_guest_timer_expirations("TIMER-EXPIRATIONS count=123"),
            Ok(123)
        );
        assert!(parse_guest_timer_expirations("TIMER-UNKNOWN").is_err());
        assert!(parse_guest_timer_expirations("TIMER-EXPIRATIONS count=-1").is_err());
    }

    #[test]
    fn parses_optional_guest_interrupt_and_rtc_samples() {
        assert_eq!(
            parse_guest_local_timer_interrupts("LOCAL-TIMER-INTERRUPTS count=70"),
            Ok(Some(70))
        );
        assert_eq!(
            parse_guest_local_timer_interrupts("LOCAL-TIMER-INTERRUPTS UNKNOWN"),
            Ok(None)
        );
        assert_eq!(
            parse_guest_rtc_epoch("RTC epoch_seconds=1792822284"),
            Ok(Some(1_792_822_284))
        );
        assert_eq!(parse_guest_rtc_epoch("RTC UNKNOWN"), Ok(None));
        assert!(parse_guest_local_timer_interrupts("LOCAL-TIMER-INTERRUPTS count=x").is_err());
        assert!(parse_guest_rtc_epoch("RTC epoch_seconds=x").is_err());
    }

    #[test]
    fn accepts_absent_ssh_keys_and_rejects_duplicated_present_keys() {
        let absent = vec![
            "IDENTITY a densevm-a RNG-RESEEDED | SSH-IDENTITY ABSENT".to_string(),
            "IDENTITY b densevm-b RNG-RESEEDED | SSH-IDENTITY ABSENT".to_string(),
        ];
        assert!(ssh_identities_are_unique(&absent));
        let unique = vec![
            "IDENTITY a densevm-a RNG-RESEEDED | SSH-IDENTITY SHA256:first".to_string(),
            "IDENTITY b densevm-b RNG-RESEEDED | SSH-IDENTITY SHA256:second".to_string(),
        ];
        assert!(ssh_identities_are_unique(&unique));
        let duplicate = vec![unique[0].clone(), unique[0].clone()];
        assert!(!ssh_identities_are_unique(&duplicate));
        let overlapping = vec![
            "IDENTITY a densevm-a RNG-RESEEDED | SSH-IDENTITY SHA256:first,SHA256:shared"
                .to_string(),
            "IDENTITY b densevm-b RNG-RESEEDED | SSH-IDENTITY SHA256:shared,SHA256:second"
                .to_string(),
        ];
        assert!(!ssh_identities_are_unique(&overlapping));
        assert!(!ssh_identities_are_unique(&[
            unique[0].clone(),
            absent[1].clone()
        ]));
        assert_eq!(
            guest_identity_value(
                "IDENTITY a densevm-a RNG-RESEEDED | SSH-IDENTITY ABSENT | DHCP-IDENTITY UNKNOWN"
            ),
            Some("IDENTITY a densevm-a RNG-RESEEDED")
        );
        assert_eq!(
            dhcp_identity_value(
                "IDENTITY a densevm-a RNG-RESEEDED | SSH-IDENTITY ABSENT | DHCP-IDENTITY UNKNOWN"
            ),
            Some("DHCP-IDENTITY UNKNOWN")
        );
    }

    #[test]
    fn accepts_only_distinct_dhcp_client_identifiers() {
        let distinct = vec![
            "DHCP-IDENTITY VERIFIED 0000112233445566778899aabbccddeeff".to_string(),
            "DHCP-IDENTITY VERIFIED 0000ffeeddccbbaa998877665544332211".to_string(),
        ];
        assert!(dhcp_identities_are_unique(&distinct));
        assert!(!dhcp_identities_are_unique(&[
            distinct[0].clone(),
            distinct[0].clone()
        ]));
        assert!(!dhcp_identities_are_unique(&[
            distinct[0].clone(),
            "DHCP-IDENTITY UNKNOWN".to_string()
        ]));
    }

    #[test]
    fn recognizes_vmgenid_rng_reactions_only_with_a_positive_event_delta() {
        assert!(vmgenid_reactions_verified(&[
            "IDENTITY a densevm-a RNG-RESEEDED | SSH-IDENTITY ABSENT | DHCP-IDENTITY UNKNOWN | VMGENID-REACTION VERIFIED event_delta=1".to_string(),
            "IDENTITY b densevm-b RNG-RESEEDED | SSH-IDENTITY ABSENT | DHCP-IDENTITY UNKNOWN | VMGENID-REACTION VERIFIED event_delta=2".to_string(),
        ]));
        assert!(!vmgenid_reactions_verified(&[
            "IDENTITY a densevm-a RNG-RESEEDED | SSH-IDENTITY ABSENT | DHCP-IDENTITY UNKNOWN | VMGENID-REACTION UNKNOWN event_delta=0".to_string(),
        ]));
    }
}

fn qemu_trace_event_supported(event: &str) -> Result<bool, String> {
    let output = Command::new("qemu-system-x86_64")
        .args(["-d", "trace:help"])
        .output()
        .map_err(|error| format!("query QEMU trace events: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "query QEMU trace events failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let events = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(events.lines().any(|line| line.trim() == event))
}

fn launch(
    out: &Path,
    name: &str,
    disk: &Path,
    backend: RamBackend<'_>,
    guest: &GuestArtifacts,
    mac_at_boot: Option<&str>,
    incoming: bool,
    vcpus: u8,
    tcg: bool,
    translation_trace: Option<(&Path, bool)>,
) -> Result<Vm, String> {
    let qmp_path = out.join(format!("{name}.qmp.sock"));
    let serial_path = out.join(format!("{name}.serial.sock"));
    let log = File::create(out.join(format!("{name}.qemu.log"))).map_err(|e| e.to_string())?;
    let mut cmd = Command::new("qemu-system-x86_64");
    let machine = format!(
        "{},accel={},mem-merge={}",
        crate::capabilities::MACHINE,
        if tcg { "tcg" } else { "kvm" },
        if guest.merge { "on" } else { "off" }
    );
    let vcpu_arg = vcpus.to_string();
    cmd.args([
        "-machine",
        &machine,
        "-cpu",
        if tcg { "max" } else { "host" },
        "-m",
        "256M",
        "-smp",
        &vcpu_arg,
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
    if let Some((path, trace_codegen_complete)) = translation_trace {
        let events = if trace_codegen_complete {
            "trace:translate_block,trace:translate_block_complete"
        } else {
            "trace:translate_block"
        };
        cmd.args(["-d", events, "-D"]).arg(path);
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

fn parse_disk_benchmark(line: &str) -> Result<Value, String> {
    if !line.starts_with("DISK-BENCH ") {
        return Err(format!("invalid disk benchmark response {line:?}"));
    }
    let mut fields = std::collections::HashMap::new();
    for field in line.split_whitespace().skip(1) {
        let (key, value) = field
            .split_once('=')
            .ok_or_else(|| format!("invalid disk benchmark field {field:?}"))?;
        fields.insert(key, value);
    }
    let parse = |key: &str| -> Result<u64, String> {
        fields
            .get(key)
            .ok_or_else(|| format!("missing disk benchmark field {key}"))?
            .parse::<u64>()
            .map_err(|error| format!("invalid disk benchmark field {key}: {error}"))
    };
    Ok(json!({
        "iterations":parse("iterations")?,
        "block_bytes":parse("block_bytes")?,
        "mean_us":parse("mean_us")?,
        "p50_us":parse("p50_us")?,
        "p95_us":parse("p95_us")?,
    }))
}

fn parse_oom_kill_delta(response: &str) -> Result<u64, String> {
    if !response.starts_with("PRESSURE-") {
        return Err(format!(
            "invalid guest memory-pressure response {response:?}"
        ));
    }
    response
        .split_whitespace()
        .find_map(|field| field.strip_prefix("oom_kill_delta="))
        .ok_or_else(|| "guest memory-pressure response omitted oom_kill_delta".to_string())?
        .parse::<u64>()
        .map_err(|error| format!("invalid guest oom_kill_delta: {error}"))
}

fn parse_guest_clock(response: &str) -> Result<(u64, u64), String> {
    if !response.starts_with("CLOCK ") {
        return Err(format!("invalid guest clock response {response:?}"));
    }
    let field = |name: &str| {
        response
            .split_whitespace()
            .find_map(|part| part.strip_prefix(&format!("{name}=")))
            .ok_or_else(|| format!("guest clock response omitted {name}"))?
            .parse::<u64>()
            .map_err(|error| format!("invalid guest clock {name}: {error}"))
    };
    Ok((field("realtime_ms")?, field("uptime_ms")?))
}

fn parse_guest_timer_expirations(response: &str) -> Result<u64, String> {
    if !response.starts_with("TIMER-EXPIRATIONS ") {
        return Err(format!("invalid guest timer response {response:?}"));
    }
    response
        .split_whitespace()
        .find_map(|part| part.strip_prefix("count="))
        .ok_or_else(|| "guest timer response omitted count".to_string())?
        .parse::<u64>()
        .map_err(|error| format!("invalid guest timer expiration count: {error}"))
}

fn parse_guest_local_timer_interrupts(response: &str) -> Result<Option<u64>, String> {
    if response.starts_with("LOCAL-TIMER-INTERRUPTS UNKNOWN") {
        return Ok(None);
    }
    if !response.starts_with("LOCAL-TIMER-INTERRUPTS ") {
        return Err(format!(
            "invalid local timer interrupt response {response:?}"
        ));
    }
    response
        .split_whitespace()
        .find_map(|part| part.strip_prefix("count="))
        .ok_or_else(|| "local timer interrupt response omitted count".to_string())?
        .parse::<u64>()
        .map(Some)
        .map_err(|error| format!("invalid local timer interrupt count: {error}"))
}

fn parse_guest_rtc_epoch(response: &str) -> Result<Option<u64>, String> {
    if response.starts_with("RTC UNKNOWN") {
        return Ok(None);
    }
    if !response.starts_with("RTC ") {
        return Err(format!("invalid guest RTC response {response:?}"));
    }
    response
        .split_whitespace()
        .find_map(|part| part.strip_prefix("epoch_seconds="))
        .ok_or_else(|| "guest RTC response omitted epoch_seconds".to_string())?
        .parse::<u64>()
        .map(Some)
        .map_err(|error| format!("invalid guest RTC epoch: {error}"))
}

fn run_disk_benchmark(serials: &mut [&mut Serial]) -> Result<Vec<Value>, String> {
    for serial in serials.iter_mut() {
        serial.send("disk-bench 16")?;
    }
    serials
        .iter_mut()
        .map(|serial| {
            let result = serial.expect("DISK-BENCH ", Duration::from_secs(15))?;
            parse_disk_benchmark(&result)
        })
        .collect()
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
    Ok(json!({
        "clones":clones,
        "template_file_resident_bytes":resident_file_bytes(ram)?,
        "cgroup_memory":cgroup_memory_snapshot(),
        "sampling":"sequential procfs reads while all clones are alive"
    }))
}

fn cgroup_memory_snapshot() -> Value {
    let Some(path) = std::env::var_os("DENSEVM_CGROUP_PATH") else {
        return json!({"status":"not-requested"});
    };
    let path = PathBuf::from(path);
    let current = match fs::read_to_string(path.join("memory.current")) {
        Ok(value) => value,
        Err(error) => {
            return json!({
                "status":"unavailable",
                "path":path,
                "reason":format!("read memory.current: {error}")
            });
        }
    };
    let stat = match fs::read_to_string(path.join("memory.stat")) {
        Ok(value) => value,
        Err(error) => {
            return json!({
                "status":"unavailable",
                "path":path,
                "reason":format!("read memory.stat: {error}")
            });
        }
    };
    match crate::metrics::parse_cgroup_memory(&current, &stat) {
        Ok(mut sample) => {
            sample["status"] = json!("measured");
            sample["path"] = json!(path);
            sample["scope"] = json!(
                "cgroup-v2 memory accounting for the isolated probe process tree; values include file cache and charged kernel memory"
            );
            sample
        }
        Err(error) => json!({
            "status":"unavailable",
            "path":path,
            "reason":error
        }),
    }
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
    verify_backing_chain(&chain, overlay, base)
}

fn verify_backing_chain(chain: &Value, overlay: &Path, base: &Path) -> Result<(), String> {
    if !chain.is_array()
        || chain.as_array().is_none_or(|items| items.len() != 2)
        || chain[0]["filename"] != overlay.to_string_lossy().as_ref()
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

fn parse_qemu_vcpu_indexes(cpus: &Value, expected: u8) -> Result<Vec<u64>, String> {
    let cpus = cpus
        .as_array()
        .ok_or_else(|| "QMP query-cpus-fast did not return an array".to_string())?;
    if cpus.len() != usize::from(expected) {
        return Err(format!(
            "QMP reported {} vCPUs, expected {expected}",
            cpus.len()
        ));
    }
    let mut indexes = cpus
        .iter()
        .map(|cpu| {
            cpu["cpu-index"]
                .as_u64()
                .ok_or_else(|| "QMP query-cpus-fast entry omitted a valid cpu-index".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    indexes.sort_unstable();
    let expected_indexes = (0..u64::from(expected)).collect::<Vec<_>>();
    if indexes != expected_indexes {
        return Err(format!(
            "QMP vCPU indexes were {indexes:?}, expected {expected_indexes:?}"
        ));
    }
    Ok(indexes)
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
    let state = qmp.execute("query-status", None)?;
    let status = state["status"]
        .as_str()
        .ok_or("QMP query-status omitted guest status after stop")?;
    if status != "paused" {
        return Err(format!("QMP stop left guest in unexpected state {status}"));
    }
    Ok(true)
}

fn resume_and_verify(qmp: &mut Qmp) -> Result<bool, String> {
    qmp.execute("cont", None)?;
    let state = qmp.execute("query-status", None)?;
    let status = state["status"]
        .as_str()
        .ok_or("QMP query-status omitted guest status after resume")?;
    if status != "running" {
        return Err(format!("QMP cont left guest in unexpected state {status}"));
    }
    Ok(true)
}

fn freeze_translation_traces(vms: &[Vm], logs: &[Option<PathBuf>]) -> Result<Vec<u64>, String> {
    if vms.len() != logs.len() {
        return Err("translation trace boundary must include one log per VM".into());
    }
    let mut offsets = Vec::with_capacity(vms.len());
    for (vm, log) in vms.iter().zip(logs) {
        let mut qmp = vm.qmp()?;
        stop_and_verify(&mut qmp)?;
        let offset = log
            .as_ref()
            .map(|path| {
                fs::metadata(path)
                    .map(|metadata| metadata.len())
                    .map_err(|error| format!("measure TCG trace {}: {error}", path.display()))
            })
            .transpose()?
            .unwrap_or(0);
        offsets.push(offset);
    }
    for vm in vms {
        let mut qmp = vm.qmp()?;
        resume_and_verify(&mut qmp)?;
    }
    Ok(offsets)
}

fn summarize_translation_phase(trace: &str, start: u64, end: u64) -> Result<Value, String> {
    let start = usize::try_from(start).map_err(|_| "TCG trace offset exceeds usize")?;
    let end = usize::try_from(end).map_err(|_| "TCG trace offset exceeds usize")?;
    if start > end || end > trace.len() {
        return Err(format!("invalid TCG trace phase range {start}..{end}"));
    }
    let phase = trace
        .get(start..end)
        .ok_or("TCG trace phase boundary is not a UTF-8 boundary")?;
    if !phase
        .lines()
        .any(|line| line.starts_with("translate_block "))
    {
        return Ok(json!({
            "status":"no_translation_events",
            "start_log_byte":start,
            "end_log_byte":end,
            "log_bytes":end-start
        }));
    }
    let summary = crate::metrics::parse_tcg_translation_summary(phase)?;
    Ok(json!({
        "status":"recorded",
        "start_log_byte":start,
        "end_log_byte":end,
        "log_bytes":end-start,
        "translation_block_events":summary.translation_block_events,
        "unique_guest_pc_count":summary.unique_guest_pc_count,
        "events_at_guest_pcs_seen_multiple_times":summary.events_at_guest_pcs_seen_multiple_times,
        "guest_pcs_seen_multiple_times":summary.guest_pcs_seen_multiple_times,
        "max_events_for_one_guest_pc":summary.max_events_for_one_guest_pc,
        "guest_pc_metric_interpretation":summary.interpretation,
        "generated_block_events":summary.generated_block_events,
        "generated_host_code_bytes_cumulative":summary.generated_host_code_bytes_cumulative,
        "translation_start_completion_event_count_delta":summary.translation_start_completion_event_count_delta,
        "generated_translation_wall_duration_us_cumulative":summary.generated_translation_wall_duration_us_cumulative,
        "generated_translation_wall_duration_us_max":summary.generated_translation_wall_duration_us_max
    }))
}

fn parse_qemu_gprs(output: &str) -> Result<std::collections::BTreeMap<String, u64>, String> {
    const GPRS: [&str; 16] = [
        "RAX", "RBX", "RCX", "RDX", "RSI", "RDI", "RSP", "RBP", "R8", "R9", "R10", "R11", "R12",
        "R13", "R14", "R15",
    ];
    let fields: Vec<_> = output.split_ascii_whitespace().collect();
    let mut registers = std::collections::BTreeMap::new();
    let mut index = 0;
    while index < fields.len() {
        let field = fields[index];
        let register = if let Some((name, value)) = field.split_once('=') {
            index += 1;
            Some((name, value))
        } else if GPRS.contains(&field) {
            match fields.get(index + 1).copied() {
                Some("=") => {
                    index += 3;
                    fields
                        .get(index.saturating_sub(1))
                        .copied()
                        .map(|value| (field, value))
                }
                Some(value) if value.starts_with('=') => {
                    index += 2;
                    Some((field, value.trim_start_matches('=')))
                }
                _ => {
                    index += 1;
                    None
                }
            }
        } else {
            index += 1;
            None
        };
        let Some((name, value)) = register else {
            continue;
        };
        if !GPRS.contains(&name) {
            continue;
        }
        let value = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
            .unwrap_or(value);
        let value = u64::from_str_radix(value, 16)
            .map_err(|error| format!("invalid QEMU {name} register value: {error}"))?;
        registers.insert(name.to_string(), value);
    }
    if registers.is_empty() {
        return Err("QEMU monitor output contained no x86-64 general-purpose registers".into());
    }
    Ok(registers)
}

fn parse_qemu_cpl(output: &str) -> Result<u8, String> {
    output
        .split_ascii_whitespace()
        .find_map(|field| field.strip_prefix("CPL="))
        .ok_or_else(|| "QEMU register output omitted CPL".to_string())?
        .parse::<u8>()
        .map_err(|error| format!("invalid QEMU CPL: {error}"))
}

fn parse_qemu_lapic_tpr(output: &str) -> Result<u8, String> {
    let value = output
        .split_ascii_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .find_map(|fields| (fields[0] == "TPR").then_some(fields[1]))
        .ok_or_else(|| "QEMU LAPIC output omitted TPR".to_string())?;
    let value = value
        .strip_prefix("0x")
        .ok_or_else(|| format!("invalid QEMU LAPIC TPR value: {value}"))?;
    u8::from_str_radix(value, 16).map_err(|error| format!("invalid QEMU LAPIC TPR: {error}"))
}

fn parse_qemu_segment_base(output: &str, segment: &str) -> Result<u64, String> {
    for line in output.lines() {
        let Some(rest) = line.trim_start().strip_prefix(segment) else {
            continue;
        };
        let fields: Vec<_> = rest.split_ascii_whitespace().collect();
        let base_index = match fields.as_slice() {
            [equals, ..] if *equals == "=" => 2,
            [selector, ..] if selector.starts_with('=') => 1,
            [..] => 1,
        };
        let Some(value) = fields.get(base_index) else {
            return Err(format!("QEMU {segment} segment output omitted its base"));
        };
        let value = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
            .unwrap_or(value);
        return u64::from_str_radix(value, 16)
            .map_err(|error| format!("invalid QEMU {segment} segment base: {error}"));
    }
    Err(format!("QEMU monitor output omitted the {segment} segment"))
}

fn parse_rtc_alarm_reply(reply: &str) -> Result<(u64, u8), String> {
    let mut fields = reply.split_whitespace();
    if fields.next() != Some("RTC-ALARM-FIRED") {
        return Err(format!("guest RTC alarm did not fire: {reply}"));
    }
    let mut events = None;
    let mut flags = None;
    for field in fields {
        if let Some(value) = field.strip_prefix("events=") {
            events = Some(value.parse::<u64>().map_err(|error| error.to_string())?);
        } else if let Some(value) = field.strip_prefix("flags=") {
            flags = Some(value.parse::<u8>().map_err(|error| error.to_string())?);
        }
    }
    Ok((
        events.ok_or_else(|| format!("RTC alarm event count missing: {reply}"))?,
        flags.ok_or_else(|| format!("RTC alarm flags missing: {reply}"))?,
    ))
}

fn rtc_alarm_arm(serial: &mut Serial) -> String {
    serial
        .send("rtc-alarm 5")
        .and_then(|()| serial.expect("RTC-ALARM-", Duration::from_secs(5)))
        .unwrap_or_else(|error| format!("RTC-ALARM-UNAVAILABLE {error}"))
}

fn rtc_alarm_collect(serial: &mut Serial) -> String {
    serial
        .expect("RTC-ALARM-", Duration::from_secs(22))
        .unwrap_or_else(|error| format!("RTC-ALARM-UNKNOWN {error}"))
}

fn rtc_alarm_observation(reply: &str) -> Value {
    match parse_rtc_alarm_reply(reply) {
        Ok((events, flags)) => json!({
            "response":reply,
            "interrupt_events":events,
            "interrupt_flags":flags,
            "alarm_flag_observed":flags & 0x20 != 0
        }),
        Err(_) => json!({
            "response":reply,
            "interrupt_events":Value::Null,
            "interrupt_flags":Value::Null,
            "alarm_flag_observed":false
        }),
    }
}

fn qemu_register_snapshot(
    qmp: &mut Qmp,
) -> Result<(std::collections::BTreeMap<String, u64>, String), String> {
    let output = qmp.execute(
        "human-monitor-command",
        Some(json!({"command-line":"info registers"})),
    )?;
    let output = output
        .as_str()
        .ok_or("QMP human-monitor-command returned non-text register output")?;
    Ok((parse_qemu_gprs(output)?, output.to_string()))
}

fn qemu_user_register_snapshot(
    qmp: &mut Qmp,
) -> Result<(std::collections::BTreeMap<String, u64>, String), String> {
    for attempt in 0..100 {
        let snapshot = qemu_register_snapshot(qmp)?;
        if parse_qemu_cpl(&snapshot.1)? == 3 {
            return Ok(snapshot);
        }
        if attempt == 99 {
            return Err("QEMU could not pause the guest in userspace after 100 attempts".into());
        }
        resume_and_verify(qmp)?;
        thread::sleep(Duration::from_millis(2));
        stop_and_verify(qmp)?;
    }
    unreachable!()
}

fn qemu_lapic_snapshot(qmp: &mut Qmp) -> Result<String, String> {
    let output = qmp.execute(
        "human-monitor-command",
        Some(json!({"command-line":"info lapic"})),
    )?;
    let output = output
        .as_str()
        .ok_or("QMP human-monitor-command returned non-text LAPIC output")?;
    if output.is_empty()
        || output.contains("unknown command")
        || output.contains("No CPU available")
    {
        return Err(format!("QEMU info lapic unavailable: {output}"));
    }
    Ok(output.to_string())
}

fn choose_lapic_tpr_targets(a_original: u8, b_original: u8) -> Result<(u8, u8), String> {
    let a_class = a_original & 0xf0;
    let b_class = b_original & 0xf0;
    let a = (1..=0x0f)
        .map(|subclass| a_class | subclass)
        .find(|value| *value != a_original)
        .ok_or("could not choose a safe clone A LAPIC TPR")?;
    let b = (1..=0x0f)
        .map(|subclass| b_class | subclass)
        .find(|value| *value != b_original && *value != a && *value != a_original)
        .ok_or("could not choose a distinct safe clone B LAPIC TPR")?;
    Ok((a, b))
}

fn set_guest_lapic_tpr(serial: &mut Serial, value: u8) -> Result<String, String> {
    let reply = serial.command(&format!("lapic-tpr {value}"), "LAPIC-TPR")?;
    let expected = format!("value={value:#04x}");
    if !reply.contains(&expected) {
        return Err(format!("guest LAPIC TPR write failed: {reply}"));
    }
    Ok(reply)
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
            vcpus: 1,
            ksm,
            divergence_pages,
            park_idle,
            divergence_all,
            guest_dir: None,
            range: false,
            skip_guest_oom: false,
            vcpu_register_state: false,
            lapic_tpr_state: false,
            rtc_alarm: false,
        },
    )
}

#[derive(Clone, Copy)]
pub struct ProbeOptions<'a> {
    pub count: u32,
    pub vcpus: u8,
    pub ksm: bool,
    pub divergence_pages: u32,
    pub park_idle: bool,
    pub divergence_all: bool,
    pub guest_dir: Option<&'a Path>,
    pub range: bool,
    pub skip_guest_oom: bool,
    pub vcpu_register_state: bool,
    pub lapic_tpr_state: bool,
    pub rtc_alarm: bool,
}

pub fn run_options(out: &Path, options: ProbeOptions<'_>) -> Result<(), String> {
    let ProbeOptions {
        count,
        vcpus,
        ksm,
        divergence_pages,
        ..
    } = options;
    if !(2..=32).contains(&count) {
        return Err(
            "the template restore probe supports clone counts from 2 through 32".to_string(),
        );
    }
    if !(1..=4).contains(&vcpus) {
        return Err("the template restore probe supports 1 through 4 vCPUs".into());
    }
    if divergence_pages > TEST_REGION_PAGES {
        return Err(format!(
            "RAM divergence must be at most {TEST_REGION_PAGES} pages"
        ));
    }
    if options.range && (options.guest_dir.is_none() || count < 3) {
        return Err("--range requires a prepared Alpine guest and at least 3 clones".into());
    }
    if options.lapic_tpr_state && !options.vcpu_register_state {
        return Err("LAPIC TPR sampling requires vCPU register sampling".into());
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
        vcpus,
        ksm,
        divergence_pages,
        park_idle,
        divergence_all,
        guest_dir,
        range,
        skip_guest_oom,
        vcpu_register_state: capture_vcpu_register_state,
        lapic_tpr_state: capture_lapic_tpr_state,
        rtc_alarm: capture_rtc_alarm,
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
    let vcpu_overhead_mib = 2 * u64::from(count) * u64::from(vcpus.saturating_sub(1)) * 64;
    let estimated_clone_budget_kib = (u64::from(count) * RAM_MIB + vcpu_overhead_mib + 512) * 1024;
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
    let identity_validation = guest.metadata["identity_validation"] == true;
    if identity_validation && range {
        return Err(
            "the identity-validation fixture uses QEMU user-mode DHCP and cannot run as a range"
                .into(),
        );
    }
    let base = out.join("base.qcow2");
    create_base(&base, &guest)?;

    let mut baseline_vms = Vec::new();
    let mut baseline_serials = Vec::new();
    let mut baseline_identity_guests = Vec::with_capacity(count as usize);
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
            vcpus,
            false,
            None,
        )?;
        let mut serial = vm.serial()?;
        serial.expect("READY", Duration::from_secs(60))?;
        baseline_identity_guests.push(prepare_identity(&mut serial, &baseline_identity)?);
        baseline_serials.push(serial);
        baseline_vms.push(vm);
    }
    let baseline_elapsed_ms = baseline_start.elapsed().as_millis();
    let baseline_pids: Vec<_> = baseline_vms.iter().map(Vm::pid).collect();
    let (baseline_cpu_sample_interval_ms, baseline_cpu_ticks) = sample_cpu_ticks(&baseline_pids)?;
    let mut baseline_serial_refs: Vec<_> = baseline_serials.iter_mut().collect();
    let (baseline_active_cpu_sample_interval_ms, baseline_active_cpu_ticks) =
        sample_active_cpu_ticks(&baseline_pids, &mut baseline_serial_refs)?;
    let baseline_disk_benchmark = if guest.disk.is_some() {
        run_disk_benchmark(&mut baseline_serial_refs)?
    } else {
        vec![Value::Null; count as usize]
    };
    let ticks_per_second = host["clock_ticks_per_second"].as_u64().unwrap_or(100);
    let mut baseline_measurements = Vec::new();
    for (index, vm) in baseline_vms.iter().enumerate() {
        let disk = out.join(format!("baseline-{index:03}.qcow2"));
        baseline_measurements.push(json!({"index":index,"pid":vm.pid(),"qemu_command":vm.command,"memory":measure(vm.pid())?,"identity_guest":baseline_identity_guests[index],"idle_cpu_ticks":baseline_cpu_ticks[index],"active_cpu_ticks":baseline_active_cpu_ticks[index],"active_cpu_percent_one_core":cpu_ticks_as_percent_of_one_core(baseline_active_cpu_ticks[index], baseline_active_cpu_sample_interval_ms, ticks_per_second),"disk_benchmark":if guest.disk.is_some(){json!(baseline_disk_benchmark[index])}else{Value::Null},"disk_allocated_bytes":allocated_bytes(&disk)?}));
    }
    let baseline_cgroup_memory = cgroup_memory_snapshot();
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
        vcpus,
        false,
        None,
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
        vcpus,
        false,
        None,
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
        vcpus,
        false,
        None,
    )?;
    let mut aq = a.qmp()?;
    let mut bq = b.qmp()?;
    let a_migration = restore(&mut aq, &state)?;
    let b_migration = restore(&mut bq, &state)?;
    let a_vcpu_indexes = parse_qemu_vcpu_indexes(&aq.execute("query-cpus-fast", None)?, vcpus)?;
    let b_vcpu_indexes = parse_qemu_vcpu_indexes(&bq.execute("query-cpus-fast", None)?, vcpus)?;
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
    let mut extra_vcpu_indexes = Vec::new();
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
            vcpus,
            false,
            None,
        )?;
        let mut qmp = vm.qmp()?;
        let migration = restore(&mut qmp, &state)?;
        extra_vcpu_indexes.push(parse_qemu_vcpu_indexes(
            &qmp.execute("query-cpus-fast", None)?,
            vcpus,
        )?);
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
    let (vcpu_register_state_verified, vcpu_register_state) = if capture_vcpu_register_state {
        let a_register_seed = 0x0a11_ce55_d15c_a11u64;
        let b_register_seed = 0x0b0b_5eed_c0de_22u64;
        let a_gs_base = 0x1111_0000u64;
        let b_gs_base = 0x2222_0000u64;
        aserial.send(&format!(
            "cpu-register-burn {a_register_seed} 10000 {a_gs_base}"
        ))?;
        let a_register_workload_armed = aserial.expect(
            &format!("CPU-REGISTER-ARMED {a_gs_base}"),
            Duration::from_secs(3),
        )?;
        bserial.send(&format!(
            "cpu-register-burn {b_register_seed} 10000 {b_gs_base}"
        ))?;
        let b_register_workload_armed = bserial.expect(
            &format!("CPU-REGISTER-ARMED {b_gs_base}"),
            Duration::from_secs(3),
        )?;
        stop_and_verify(&mut aq)?;
        stop_and_verify(&mut bq)?;
        let (a_vcpu_registers, a_vcpu_register_text) = qemu_user_register_snapshot(&mut aq)?;
        let (b_vcpu_registers, b_vcpu_register_text) = qemu_user_register_snapshot(&mut bq)?;
        let a_lapic_text = qemu_lapic_snapshot(&mut aq)?;
        let b_lapic_text = qemu_lapic_snapshot(&mut bq)?;
        let a_gs_base_observed = parse_qemu_segment_base(&a_vcpu_register_text, "GS")?;
        let b_gs_base_observed = parse_qemu_segment_base(&b_vcpu_register_text, "GS")?;
        let differing_vcpu_registers: Vec<_> = a_vcpu_registers
            .iter()
            .filter_map(|(name, a_value)| {
                b_vcpu_registers
                    .get(name)
                    .filter(|b_value| *b_value != a_value)
                    .map(|_| name.clone())
            })
            .collect();
        resume_and_verify(&mut aq)?;
        resume_and_verify(&mut bq)?;
        let a_register_workload_done =
            aserial.expect("CPU-REGISTER-DONE", Duration::from_secs(12))?;
        let b_register_workload_done =
            bserial.expect("CPU-REGISTER-DONE", Duration::from_secs(12))?;
        let (lapic_tpr_state_verified, lapic_tpr_state) = if capture_lapic_tpr_state {
            let a_original = parse_qemu_lapic_tpr(&a_lapic_text)?;
            let b_original = parse_qemu_lapic_tpr(&b_lapic_text)?;
            let (a_target, b_target) = choose_lapic_tpr_targets(a_original, b_original)?;

            let a_write_reply = set_guest_lapic_tpr(&mut aserial, a_target)?;
            stop_and_verify(&mut aq)?;
            stop_and_verify(&mut bq)?;
            let a_after_a_write = qemu_lapic_snapshot(&mut aq)?;
            let b_after_a_write = qemu_lapic_snapshot(&mut bq)?;
            let a_after_a_write_tpr = parse_qemu_lapic_tpr(&a_after_a_write)?;
            let b_after_a_write_tpr = parse_qemu_lapic_tpr(&b_after_a_write)?;
            resume_and_verify(&mut aq)?;
            resume_and_verify(&mut bq)?;

            let b_write_reply = set_guest_lapic_tpr(&mut bserial, b_target)?;
            stop_and_verify(&mut aq)?;
            stop_and_verify(&mut bq)?;
            let a_after_b_write = qemu_lapic_snapshot(&mut aq)?;
            let b_after_b_write = qemu_lapic_snapshot(&mut bq)?;
            let a_after_b_write_tpr = parse_qemu_lapic_tpr(&a_after_b_write)?;
            let b_after_b_write_tpr = parse_qemu_lapic_tpr(&b_after_b_write)?;
            resume_and_verify(&mut aq)?;
            resume_and_verify(&mut bq)?;

            let a_restore_reply = set_guest_lapic_tpr(&mut aserial, a_original)?;
            let b_restore_reply = set_guest_lapic_tpr(&mut bserial, b_original)?;
            stop_and_verify(&mut aq)?;
            stop_and_verify(&mut bq)?;
            let a_restored = qemu_lapic_snapshot(&mut aq)?;
            let b_restored = qemu_lapic_snapshot(&mut bq)?;
            let a_restored_tpr = parse_qemu_lapic_tpr(&a_restored)?;
            let b_restored_tpr = parse_qemu_lapic_tpr(&b_restored)?;
            resume_and_verify(&mut aq)?;
            resume_and_verify(&mut bq)?;

            let verified = a_after_a_write_tpr == a_target
                && b_after_a_write_tpr == b_original
                && (a_after_b_write_tpr == a_target || a_after_b_write_tpr == a_original)
                && a_after_b_write_tpr != b_target
                && b_after_b_write_tpr == b_target
                && a_restored_tpr == a_original
                && b_restored_tpr == b_original;
            (
                Some(verified),
                json!({
                    "status":if verified {"verified"} else {"failed"},
                    "method":"guest attempts x2APIC TPR MSR then xAPIC TPR MMIO; QMP info lapic snapshots verify the register writes",
                    "a":{"original_tpr":a_original,"target_tpr":a_target,"after_a_write":a_after_a_write,"after_b_write":a_after_b_write,"restored":a_restored,"write_reply":a_write_reply,"restore_reply":a_restore_reply},
                    "b":{"original_tpr":b_original,"target_tpr":b_target,"after_a_write":b_after_a_write,"after_b_write":b_after_b_write,"restored":b_restored,"write_reply":b_write_reply,"restore_reply":b_restore_reply},
                    "observed":{"a_after_a_write":a_after_a_write_tpr,"b_after_a_write":b_after_a_write_tpr,"a_after_b_write":a_after_b_write_tpr,"b_after_b_write":b_after_b_write_tpr,"a_restored":a_restored_tpr,"b_restored":b_restored_tpr},
                    "b_write_reply":b_write_reply
                }),
            )
        } else {
            (
                None,
                json!({"status":"not-requested","how_to_enable":"densevm probe --lapic-tpr-state"}),
            )
        };
        let verified = a_register_workload_armed.starts_with("CPU-REGISTER-ARMED")
            && b_register_workload_armed.starts_with("CPU-REGISTER-ARMED")
            && a_register_workload_done.contains(&format!("seed={a_register_seed}"))
            && b_register_workload_done.contains(&format!("seed={b_register_seed}"))
            && a_gs_base_observed == a_gs_base
            && b_gs_base_observed == b_gs_base
            && a_gs_base_observed != b_gs_base_observed
            && !differing_vcpu_registers.is_empty()
            && lapic_tpr_state_verified.unwrap_or(true);
        (
            Some(verified),
            json!({
                "status":if verified {"verified"} else {"failed"},
                "capture":"QMP human-monitor-command info registers and info lapic while each clone was paused",
                "workload":"guest CPU arithmetic with clone-specific seed and user GS base, stopped after both workloads announced readiness",
                "a":{"seed":a_register_seed,"gs_base_expected":a_gs_base,"gs_base_observed":a_gs_base_observed,"registers":a_vcpu_registers,"monitor_output":a_vcpu_register_text,"lapic_output":a_lapic_text,"workload_done":a_register_workload_done},
                "b":{"seed":b_register_seed,"gs_base_expected":b_gs_base,"gs_base_observed":b_gs_base_observed,"registers":b_vcpu_registers,"monitor_output":b_vcpu_register_text,"lapic_output":b_lapic_text,"workload_done":b_register_workload_done},
                "differing_general_purpose_registers":differing_vcpu_registers,
                "lapic_tpr_state":lapic_tpr_state
            }),
        )
    } else {
        (
            None,
            json!({"status":"not-requested","how_to_enable":"densevm probe --vcpu-register-state"}),
        )
    };
    let (rtc_alarm_verified, rtc_alarm) = if capture_rtc_alarm {
        let a_control_armed = rtc_alarm_arm(&mut aserial);
        let b_control_armed = rtc_alarm_arm(&mut bserial);
        let (a_control_reply, b_control_reply) = if a_control_armed.starts_with("RTC-ALARM-ARMED")
            && b_control_armed.starts_with("RTC-ALARM-ARMED")
        {
            thread::sleep(Duration::from_secs(7));
            (
                rtc_alarm_collect(&mut aserial),
                rtc_alarm_collect(&mut bserial),
            )
        } else {
            (a_control_armed.clone(), b_control_armed.clone())
        };

        let a_paused_armed = rtc_alarm_arm(&mut aserial);
        let b_paused_armed = rtc_alarm_arm(&mut bserial);
        let (a_paused_reply, b_paused_reply) = if a_paused_armed.starts_with("RTC-ALARM-ARMED")
            && b_paused_armed.starts_with("RTC-ALARM-ARMED")
        {
            stop_and_verify(&mut aq)?;
            stop_and_verify(&mut bq)?;
            thread::sleep(Duration::from_secs(7));
            resume_and_verify(&mut aq)?;
            resume_and_verify(&mut bq)?;
            (
                rtc_alarm_collect(&mut aserial),
                rtc_alarm_collect(&mut bserial),
            )
        } else {
            (a_paused_armed.clone(), b_paused_armed.clone())
        };
        let a_control = rtc_alarm_observation(&a_control_reply);
        let b_control = rtc_alarm_observation(&b_control_reply);
        let a_paused = rtc_alarm_observation(&a_paused_reply);
        let b_paused = rtc_alarm_observation(&b_paused_reply);
        let verified = a_paused["alarm_flag_observed"] == true
            && b_paused["alarm_flag_observed"] == true
            && a_paused["interrupt_events"]
                .as_u64()
                .is_some_and(|events| events > 0)
            && b_paused["interrupt_events"]
                .as_u64()
                .is_some_and(|events| events > 0);
        (
            Some(verified),
            json!({
                "status":if verified {"paused_delivery_verified"} else {"paused_delivery_failed_or_unavailable"},
                "method":"compare an alarm while both guests run against a second alarm armed before a seven-second QMP pause",
                "alarm_delay_seconds":5,
                "pause_duration_seconds":7,
                "rtc_alarm_flag":"RTC_AF (0x20)",
                "control_running":{"a_armed":a_control_armed,"a":a_control,"b_armed":b_control_armed,"b":b_control},
                "paused":{"a_armed":a_paused_armed,"a":a_paused,"b_armed":b_paused_armed,"b":b_paused}
            }),
        )
    } else {
        (
            None,
            json!({"status":"not-requested","how_to_enable":"densevm probe --rtc-alarm"}),
        )
    };
    let mut guest_identities = vec![a_identity_guest.clone(), b_identity_guest.clone()];
    let mut clone_identity_guests = vec![a_identity_guest.clone(), b_identity_guest.clone()];
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
        clone_identity_guests.push(identity_guest.clone());
        vmgenid_unique &= !all_vmgenids.iter().any(|existing| existing == vmgenid);
        all_vmgenids.push(vmgenid.clone());
    }
    let mut all_ssh_identity_guests = baseline_identity_guests.clone();
    all_ssh_identity_guests.extend(clone_identity_guests.iter().cloned());
    let ssh_host_keys_unique = ssh_identities_are_unique(&all_ssh_identity_guests);
    let ssh_host_keys_present = all_ssh_identity_guests
        .iter()
        .any(|reply| ssh_identity_value(reply).is_some_and(|value| value != "ABSENT"));
    let a_mac_guest = aserial.command("mac-read", "MAC ")?;
    let b_mac_guest = bserial.command("mac-read", "MAC ")?;
    let mut dhcp_identity_responses = Vec::new();
    let mut range_before_verified = !range;
    let mut range_network_benchmark = None;
    if range || identity_validation {
        aserial.command("network-set 10.77.0.10", "NETWORK ")?;
        bserial.command("network-set 10.77.0.11", "NETWORK ")?;
        for (_, _, serial, index, _, _, _) in &mut extra_clones {
            serial.command(&format!("network-set 10.77.0.{}", *index + 10), "NETWORK ")?;
        }
    }
    if identity_validation {
        dhcp_identity_responses
            .push(aserial.command("dhcp-identity-probe", "DHCP-IDENTITY VERIFIED ")?);
        dhcp_identity_responses
            .push(bserial.command("dhcp-identity-probe", "DHCP-IDENTITY VERIFIED ")?);
        for (_, _, serial, _, _, _, _) in &mut extra_clones {
            dhcp_identity_responses
                .push(serial.command("dhcp-identity-probe", "DHCP-IDENTITY VERIFIED ")?);
        }
    }
    let dhcp_identity_unique =
        identity_validation && dhcp_identities_are_unique(&dhcp_identity_responses);
    let identity_validation_verified = !identity_validation
        || (ssh_host_keys_present && ssh_host_keys_unique && dhcp_identity_unique);
    if range {
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
    let (
        process_isolation,
        process_isolation_status,
        process_isolation_scope,
        pair_process_isolation,
    ) = if guest.disk.is_some() {
        aserial.command("process-start", "PROCESS-STARTED")?;
        let a_process_isolated = expect_value(&mut aserial, "process-count", "PROCESS-COUNT 1")?;
        let b_process_isolated = expect_value(&mut bserial, "process-count", "PROCESS-COUNT 0")?;
        let mut extra_processes_isolated = true;
        for (_, _, serial, _, _, _, _) in &mut extra_clones {
            let isolated = expect_value(serial, "process-count", "PROCESS-COUNT 0")?;
            extra_processes_isolated &= isolated;
        }
        let pair_verified = a_process_isolated && b_process_isolated;
        let verified = pair_verified && extra_processes_isolated;
        (
            Some(verified),
            if verified { "verified" } else { "failed" },
            "clone-000 started /bin/sleep 137; every other live clone reported zero matching processes",
            Some(pair_verified),
        )
    } else {
        (
            None,
            "not-requested",
            "not sampled for the synthetic initramfs guest profile",
            None,
        )
    };
    let guest_oom_response = if skip_guest_oom {
        "PRESSURE-SKIPPED".to_string()
    } else {
        aserial.command("memory-pressure", "PRESSURE-")?
    };
    let guest_oom_kill_delta = if skip_guest_oom {
        0
    } else {
        parse_oom_kill_delta(&guest_oom_response)?
    };
    let guest_oom_killed_worker = !skip_guest_oom
        && guest_oom_response.starts_with("PRESSURE-OOM ")
        && guest_oom_kill_delta > 0;
    let guests_survive_oom_pressure = if skip_guest_oom {
        false
    } else {
        let mut survived = expect_value(&mut aserial, "ping", "PONG")?
            && expect_value(&mut bserial, "ping", "PONG")?;
        for (_, _, serial, _, _, _, _) in &mut extra_clones {
            survived &= expect_value(serial, "ping", "PONG")?;
        }
        survived
    };
    let guest_oom_isolation_verified = guest_oom_killed_worker && guests_survive_oom_pressure;
    let guest_oom_check_passed = skip_guest_oom || guest_oom_isolation_verified;
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
    let clone_disk_benchmark = if guest.disk.is_some() {
        let mut disk_serial_refs: Vec<_> = vec![&mut aserial, &mut bserial];
        for (_, _, serial, _, _, _, _) in &mut extra_clones {
            disk_serial_refs.push(serial);
        }
        run_disk_benchmark(&mut disk_serial_refs)?
    } else {
        vec![Value::Null; count as usize]
    };
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
        aserial.command("timer-start", "TIMER-STARTED ")?;
        bserial.command("timer-start", "TIMER-STARTED ")?;
        for (_, _, serial, _, _, _, _) in &mut extra_clones {
            serial.command("timer-start", "TIMER-STARTED ")?;
        }
        thread::sleep(Duration::from_millis(50));
        let mut guest_timer_before = vec![
            aserial.command("timer-read", "TIMER-EXPIRATIONS ")?,
            bserial.command("timer-read", "TIMER-EXPIRATIONS ")?,
        ];
        for (_, _, serial, _, _, _, _) in &mut extra_clones {
            guest_timer_before.push(serial.command("timer-read", "TIMER-EXPIRATIONS ")?);
        }
        let mut guest_interrupts_before = vec![
            aserial.command("local-timer-interrupts-read", "LOCAL-TIMER-INTERRUPTS ")?,
            bserial.command("local-timer-interrupts-read", "LOCAL-TIMER-INTERRUPTS ")?,
        ];
        let mut guest_rtc_before = vec![
            aserial.command("rtc-read", "RTC ")?,
            bserial.command("rtc-read", "RTC ")?,
        ];
        for (_, _, serial, _, _, _, _) in &mut extra_clones {
            guest_interrupts_before
                .push(serial.command("local-timer-interrupts-read", "LOCAL-TIMER-INTERRUPTS ")?);
            guest_rtc_before.push(serial.command("rtc-read", "RTC ")?);
        }
        let mut guest_clock_before = vec![
            aserial.command("clock-read", "CLOCK ")?,
            bserial.command("clock-read", "CLOCK ")?,
        ];
        for (_, _, serial, _, _, _, _) in &mut extra_clones {
            guest_clock_before.push(serial.command("clock-read", "CLOCK ")?);
        }
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
        let resume_started = Instant::now();
        resumed_statuses.push(resume_and_verify(&mut aq)?);
        resumed_statuses.push(resume_and_verify(&mut bq)?);
        for qmp in &mut extra_qmps {
            resumed_statuses.push(resume_and_verify(qmp)?);
        }
        let resume_qmp_elapsed_ms = resume_started.elapsed().as_millis();
        let mut guest_clock_after = vec![
            aserial.command("clock-read", "CLOCK ")?,
            bserial.command("clock-read", "CLOCK ")?,
        ];
        for (_, _, serial, _, _, _, _) in &mut extra_clones {
            guest_clock_after.push(serial.command("clock-read", "CLOCK ")?);
        }
        let mut guest_timer_after = vec![
            aserial.command("timer-read", "TIMER-EXPIRATIONS ")?,
            bserial.command("timer-read", "TIMER-EXPIRATIONS ")?,
        ];
        for (_, _, serial, _, _, _, _) in &mut extra_clones {
            guest_timer_after.push(serial.command("timer-read", "TIMER-EXPIRATIONS ")?);
        }
        let mut guest_interrupts_after = vec![
            aserial.command("local-timer-interrupts-read", "LOCAL-TIMER-INTERRUPTS ")?,
            bserial.command("local-timer-interrupts-read", "LOCAL-TIMER-INTERRUPTS ")?,
        ];
        let mut guest_rtc_after = vec![
            aserial.command("rtc-read", "RTC ")?,
            bserial.command("rtc-read", "RTC ")?,
        ];
        for (_, _, serial, _, _, _, _) in &mut extra_clones {
            guest_interrupts_after
                .push(serial.command("local-timer-interrupts-read", "LOCAL-TIMER-INTERRUPTS ")?);
            guest_rtc_after.push(serial.command("rtc-read", "RTC ")?);
        }
        let guest_timer_deltas = guest_timer_before
            .iter()
            .zip(&guest_timer_after)
            .enumerate()
            .map(|(index, (before, after))| {
                let before = parse_guest_timer_expirations(before)?;
                let after = parse_guest_timer_expirations(after)?;
                Ok(json!({
                    "clone_index":index,
                    "expirations_before_pause":before,
                    "expirations_after_resume":after,
                    "observed_delta":after.checked_sub(before)
                }))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let guest_clock_deltas = guest_clock_before
            .iter()
            .zip(&guest_clock_after)
            .enumerate()
            .map(|(index, (before, after))| {
                let before = parse_guest_clock(before)?;
                let after = parse_guest_clock(after)?;
                let local_interrupts_before =
                    parse_guest_local_timer_interrupts(&guest_interrupts_before[index])?;
                let local_interrupts_after =
                    parse_guest_local_timer_interrupts(&guest_interrupts_after[index])?;
                let rtc_before = parse_guest_rtc_epoch(&guest_rtc_before[index])?;
                let rtc_after = parse_guest_rtc_epoch(&guest_rtc_after[index])?;
                Ok(json!({
                    "clone_index":index,
                    "realtime_before_ms":before.0,
                    "realtime_after_ms":after.0,
                    "realtime_delta_ms":after.0 as i128 - before.0 as i128,
                    "uptime_before_ms":before.1,
                    "uptime_after_ms":after.1,
                    "uptime_delta_ms":after.1 as i128 - before.1 as i128,
                    "local_timer_interrupts_before":local_interrupts_before,
                    "local_timer_interrupts_after":local_interrupts_after,
                    "local_timer_interrupts_delta":local_interrupts_before.zip(local_interrupts_after).and_then(|(before, after)| after.checked_sub(before)),
                    "rtc_epoch_before_seconds":rtc_before,
                    "rtc_epoch_after_seconds":rtc_after,
                    "rtc_epoch_delta_seconds":rtc_before.zip(rtc_after).and_then(|(before, after)| after.checked_sub(before))
                }))
            })
            .collect::<Result<Vec<_>, String>>()?;
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
            "resume_qmp_elapsed_ms":resume_qmp_elapsed_ms,
            "guest_clock_samples":guest_clock_deltas,
            "guest_timerfd_expiration_samples":guest_timer_deltas,
            "guests_responsive_after_resume":guests_responsive_after_resume,
            "guest_clock_scope":"wall time, /proc/uptime, /proc/interrupts LOC counters, and /sys/class/rtc/rtc0/since_epoch sampled around QMP pause; timerfd expiration counters sampled before pause and after resume; post-resume reads include execution after resume and do not isolate events during pause"
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
        let identity_reply = serial.command("identity-read", "IDENTITY ")?;
        surviving_clones_verified &= expect_value(serial, "ping", "PONG")?
            && guest_identity_value(identity).is_some_and(|expected| expected == identity_reply)
            && serial.command("disk-read", "DISK ")? == format!("DISK {}", 10 + *index)
            && serial.command("ram-read", "RAM ")? == format!("RAM {expected_ram}");
    }
    let guest_crash_requested = !extra_clones.is_empty();
    let (guest_crash_observed, guest_crash_survivors_verified, guest_crash_qemu_alive) =
        if let Some((crash_vm, _, crash_serial, _, _, _, _)) = extra_clones.first_mut() {
            crash_serial.send("kernel-crash")?;
            let crash_observed =
                crash_serial.observe_raw("Kernel panic", Duration::from_secs(8))?;
            let qemu_alive = crash_vm
                .child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_none()
                && crash_vm.qmp()?.execute("query-status", None).is_ok();
            let mut survivors = expect_value(&mut bserial, "ping", "PONG")?;
            for (_, _, serial, _, _, _, _) in extra_clones.iter_mut().skip(1) {
                survivors &= expect_value(serial, "ping", "PONG")?;
            }
            (crash_observed, survivors, qemu_alive)
        } else {
            (false, false, false)
        };
    let guest_crash_isolation_verified = guest_crash_requested
        && guest_crash_observed
        && guest_crash_survivors_verified
        && guest_crash_qemu_alive;
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
                    "disk_benchmark": if guest.disk.is_some(){json!(clone_disk_benchmark[*index as usize])}else{Value::Null},
                    "overlay_allocated_bytes": allocated_bytes(&overlays[*index as usize])?,
                    "identity_guest": identity_guest,
                    "mac_guest": mac_guest,
                    "vmgenid": vmgenid,
                    "vcpu_indexes": extra_vcpu_indexes[*index as usize - 2],
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
    let guest_crash_result = json!({
        "requested":guest_crash_requested,
        "crash_observed":guest_crash_observed,
        "other_clones_survived":guest_crash_survivors_verified,
        "crashed_clone_qemu_alive":guest_crash_qemu_alive,
        "isolation_verified":guest_crash_isolation_verified,
        "status":if guest_crash_requested {"performed"} else {"not run; requires at least three clones"}
    });
    let mut result = json!({
        "experiment":"two-clone-cow-probe",
        "host":host,
        "guest":guest.metadata,
        "distribution_checkpoint":distribution,
        "qemu_version":output(Command::new("qemu-system-x86_64").arg("--version"))?.lines().next(),
        "kernel_version":output(Command::new("uname").arg("-r"))?.trim(),
        "guest_ram_mib":RAM_MIB,
        "guest_vcpus":vcpus,
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
        "baseline":{"count":count,"vms":baseline_measurements,"cgroup_memory_at_measurement":baseline_cgroup_memory,"time_to_count_ready_ms":baseline_elapsed_ms,"idle_cpu_sample_interval_ms":baseline_cpu_sample_interval_ms,"active_cpu_sample_interval_ms":baseline_active_cpu_sample_interval_ms},
        "template":{"qemu_command":template_command,"memory_file_bytes":RAM_BYTES,"memory_file_allocated_bytes":allocated_bytes(&ram)?,"memory_file_resident_bytes":memory_file_resident_bytes,"memory_sha256":ram_hash_after,"vmgenid":template_vmgenid,"machine_state_allocated_bytes":allocated_bytes(&state)?,"migration":template_migration},
        "disk_storage":{"base_allocated_bytes":allocated_bytes(&base)?,"total_clone_overlay_allocated_bytes":total_overlay_allocated_bytes},
        "clones":{"count":count,"idle_cpu_sample_interval_ms":clone_cpu_sample_interval_ms,"a":{"pid":a.pid(),"qemu_command":a.command,"memory":before_a,"ram_mapping":before_ram_a,"idle_cpu_ticks":clone_idle_cpu_ticks[0],"after_ram_write_memory":after_ram_a,"after_ram_write_ram_mapping":after_ram_mapping_a,"mac_guest":a_mac_guest,"identity_guest":a_identity_guest,"vmgenid":a_vmgenid,"vcpu_indexes":a_vcpu_indexes,"overlay_initial_allocated_bytes":overlay_before[0],"overlay_final_allocated_bytes":overlay_after[0],"overlay_after_full_write_allocated_bytes":a_overlay_after_full_write,"migration":a_migration},
                  "b":{"pid":b.pid(),"qemu_command":b.command,"memory":before_b,"ram_mapping":before_ram_b,"idle_cpu_ticks":clone_idle_cpu_ticks[1],"after_a_ram_write_memory":after_ram_b,"after_a_ram_write_ram_mapping":after_ram_mapping_b,"mac_guest":b_mac_guest,"identity_guest":b_identity_guest,"vmgenid":b_vmgenid,"vcpu_indexes":b_vcpu_indexes,"overlay_initial_allocated_bytes":overlay_before[1],"overlay_final_allocated_bytes":overlay_after[1],"migration":b_migration},
                  "extra":extra_measurements,
                  "time_to_count_resumed_ms":clone_elapsed_ms},
        "ram_isolation":ram_isolation,"disk_isolation":disk_isolation,"extra_clone_isolation":extra_guest_isolation,"full_disk_isolation":full_disk_isolation,"b_survives_guest_reboot":b_survives_guest_reboot,"template_ram_unchanged":template_ram_unchanged,"backing_verified":true,"mac_unique":all_macs_unique,"identity_unique":identity_unique,"vmgenid_unique":vmgenid_unique,"b_survives_a_kill":b_survives_a_kill,"parking_verified":parking_verified,"parking":parking_details,
        "guest_crash":guest_crash_result,
        "vcpu_register_state":vcpu_register_state,
        "vcpu_register_state_verified":vcpu_register_state_verified,
        "rtc_alarm":rtc_alarm,
        "rtc_alarm_verified":rtc_alarm_verified
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
    result["clones"]["a"]["disk_benchmark"] = if guest.disk.is_some() {
        json!(clone_disk_benchmark[0])
    } else {
        Value::Null
    };
    result["clones"]["b"]["disk_benchmark"] = if guest.disk.is_some() {
        json!(clone_disk_benchmark[1])
    } else {
        Value::Null
    };
    result["active_cpu_workload"] = json!({"guest_command":format!("cpu-burn {ACTIVE_CPU_BURN_MS}"),"sample_interval_ms":clone_active_cpu_sample_interval_ms,"cpu_ticks_units":"raw /proc/<pid>/stat scheduler ticks","ticks_per_second":ticks_per_second});
    result["disk_io_benchmark"] = if guest.disk.is_some() {
        json!({"guest_command":"disk-bench 16","scope":"per-guest ext4 file; each 4 KiB write followed by fdatasync; latencies measured inside guest"})
    } else {
        Value::Null
    };
    result["range"] = json!({"enabled":range,"backend":guest.netdev,"attacker_index":if range {Some(count-1)} else {None},"target_count":if range {Some(count-1)} else {None},"connectivity_before":range_before_verified,"connectivity_after_target_kill":range_after_verified,"network_benchmark":range_network_benchmark});
    result["process_isolation"] = json!(process_isolation);
    result["process_isolation_status"] = json!(process_isolation_status);
    result["process_isolation_scope"] = json!(process_isolation_scope);
    result["process_isolation_pair_verified"] = json!(pair_process_isolation);
    result["guest_oom"] = json!({
        "requested":!skip_guest_oom,
        "status":if skip_guest_oom {"skipped"} else if guest_oom_isolation_verified {"verified"} else {"failed"},
        "total_worker_allocation_bytes":8 * 64 * 1024 * 1024u64,
        "response":guest_oom_response,
        "oom_kill_delta":guest_oom_kill_delta,
        "worker_oom_killed":(!skip_guest_oom).then_some(guest_oom_killed_worker),
        "all_guests_survived":(!skip_guest_oom).then_some(guests_survive_oom_pressure),
        "isolation_verified":(!skip_guest_oom).then_some(guest_oom_isolation_verified)
    });
    result["ssh_host_keys"] = json!({
        "present":ssh_host_keys_present,
        "unique":ssh_host_keys_unique,
        "identity_responses":all_ssh_identity_guests
    });
    result["dhcp_identity"] = json!({
        "status":if identity_validation {if dhcp_identity_unique {"VERIFIED"} else {"FAILED"}} else {"UNKNOWN"},
        "unique":identity_validation.then_some(dhcp_identity_unique),
        "method":if identity_validation {"BusyBox udhcpc lease plus AF_PACKET observation of DHCP option 61"} else {"not measured"},
        "guest_responses":if identity_validation {dhcp_identity_responses} else {all_ssh_identity_guests.iter().filter_map(|reply| dhcp_identity_value(reply)).map(str::to_string).collect()},
        "reason":if identity_validation {"each clone sent its machine-id-derived option 61 and received a DHCP lease"} else {"the tested guest does not run a DHCP client; unique MAC and static IP values do not prove DHCP client identity"}
    });
    result["identity_validation_verified"] = json!(identity_validation_verified);
    let vmgenid_guest_reaction_verified = vmgenid_reactions_verified(&clone_identity_guests);
    result["vmgenid_guest_reaction"] = json!({
        "status":if vmgenid_guest_reaction_verified {"VERIFIED"} else {"UNKNOWN"},
        "measurement":"positive delta in the Linux kernel log message emitted by add_vmfork_randomness before host entropy injection",
        "clone_responses":clone_identity_guests.iter().filter_map(|reply| reply.split(" | ").nth(3)).collect::<Vec<_>>()
    });
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
        && ssh_host_keys_unique
        && identity_validation_verified
        && vmgenid_unique
        && extra_guest_isolation
        && full_disk_isolation
        && b_survives_guest_reboot
        && b_survives_a_kill
        && parking_verified
        && divergence_all_verified
        && process_isolation != Some(false)
        && guest_oom_check_passed
        && (!guest_crash_requested || guest_crash_isolation_verified)
        && surviving_clones_verified
        && a_reboot_observed
        && range_before_verified
        && range_after_verified
        && vcpu_register_state_verified != Some(false)
        && rtc_alarm_verified != Some(false))
    {
        return Err("one or more isolation checks failed; see result.json".into());
    }
    Ok(())
}

fn prepare_identity(serial: &mut Serial, identity: &CloneIdentity) -> Result<String, String> {
    let vmgenid_reaction = serial.command("vmgenid-reaction-read", "VMGENID-REACTION ")?;
    serial.command(
        &format!("entropy-add {}", identity.entropy_hex()),
        "RNG-RESEEDED",
    )?;
    serial.command(
        &format!("identity-set {}", identity.machine_id),
        "IDENTITY-SET ",
    )?;
    let guest_identity = serial.command("identity-read", "IDENTITY ")?;
    let ssh_identity = serial.command("ssh-identity-read", "SSH-IDENTITY ")?;
    let dhcp_identity = serial.command("dhcp-identity-read", "DHCP-IDENTITY ")?;
    Ok(format!(
        "{guest_identity} | {ssh_identity} | {dhcp_identity} | {vmgenid_reaction}"
    ))
}

fn identity_matches(reply: &str, identity: &str) -> bool {
    let Some((guest_identity, ssh_identity)) = reply.split_once(" | ") else {
        return false;
    };
    guest_identity
        == format!(
            "IDENTITY {identity} densevm-{} RNG-RESEEDED",
            &identity[..12]
        )
        && ssh_identity.starts_with("SSH-IDENTITY ")
}

fn guest_identity_value(reply: &str) -> Option<&str> {
    reply
        .split_once(" | ")
        .map(|(guest_identity, _)| guest_identity)
}

fn ssh_identity_value(reply: &str) -> Option<&str> {
    reply.split(" | ").nth(1)?.strip_prefix("SSH-IDENTITY ")
}

fn dhcp_identity_value(reply: &str) -> Option<&str> {
    reply.split(" | ").nth(2)
}

fn ssh_identities_are_unique(replies: &[String]) -> bool {
    let Some(values) = replies
        .iter()
        .map(|reply| ssh_identity_value(reply))
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    if values.iter().all(|value| *value == "ABSENT") {
        return true;
    }
    if values.iter().any(|value| *value == "ABSENT") {
        return false;
    }
    let mut fingerprints = std::collections::HashSet::new();
    for value in values {
        let entries: Vec<_> = value.split(',').collect();
        if entries.is_empty()
            || entries.iter().any(|fingerprint| {
                !fingerprint.starts_with("SHA256:") || !fingerprints.insert(*fingerprint)
            })
        {
            return false;
        }
    }
    true
}

fn dhcp_identities_are_unique(replies: &[String]) -> bool {
    let mut identifiers = std::collections::HashSet::new();
    !replies.is_empty()
        && replies.iter().all(|reply| {
            let Some(identifier) = reply.strip_prefix("DHCP-IDENTITY VERIFIED ") else {
                return false;
            };
            identifier.len() == 34
                && identifier.bytes().all(|byte| byte.is_ascii_hexdigit())
                && identifiers.insert(identifier.to_ascii_lowercase())
        })
}

fn vmgenid_reactions_verified(replies: &[String]) -> bool {
    !replies.is_empty()
        && replies.iter().all(|reply| {
            reply
                .split(" | ")
                .nth(3)
                .and_then(|field| field.strip_prefix("VMGENID-REACTION VERIFIED event_delta="))
                .and_then(|delta| delta.parse::<u64>().ok())
                .is_some_and(|delta| delta > 0)
        })
}

pub fn run_baseline(
    out: &Path,
    count: u32,
    tcg: bool,
    trace_translations: bool,
) -> Result<(), String> {
    if count == 0 || count > 32 {
        return Err("baseline count must be between 1 and 32".to_string());
    }
    if trace_translations && !tcg {
        return Err("translation tracing requires the TCG accelerator".into());
    }
    if trace_translations && count > 2 {
        return Err("translation tracing is limited to at most two VMs to bound log size".into());
    }
    let trace_codegen_complete =
        trace_translations && tcg && qemu_trace_event_supported("translate_block_complete")?;
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
    let mut translation_logs = Vec::with_capacity(count as usize);
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
        let translation_log = if trace_translations {
            Some(out.join(format!("{name}.tcg-trace.log")))
        } else {
            None
        };
        let vm = launch(
            &out,
            &name,
            &disk,
            RamBackend::Anonymous,
            &guest,
            Some(&mac),
            false,
            1,
            tcg,
            translation_log
                .as_deref()
                .map(|path| (path, trace_codegen_complete)),
        )?;
        let mut serial = vm.serial()?;
        serial.expect("READY", Duration::from_secs(60))?;
        serials.push(serial);
        translation_logs.push(translation_log);
        vms.push(vm);
    }
    let elapsed = start.elapsed().as_millis();
    let pids: Vec<_> = vms.iter().map(Vm::pid).collect();
    let ready_trace_offsets = if trace_translations {
        Some(freeze_translation_traces(&vms, &translation_logs)?)
    } else {
        None
    };
    let (idle_cpu_sample_interval_ms, idle_cpu_ticks) = sample_cpu_ticks(&pids)?;
    let idle_trace_offsets = if trace_translations {
        Some(freeze_translation_traces(&vms, &translation_logs)?)
    } else {
        None
    };
    let mut serial_refs: Vec<_> = serials.iter_mut().collect();
    let (active_cpu_sample_interval_ms, active_cpu_ticks) =
        sample_active_cpu_ticks(&pids, &mut serial_refs)?;
    let mut final_trace_offsets = Vec::with_capacity(vms.len());
    if trace_translations {
        for (vm, log) in vms.iter().zip(&translation_logs) {
            let mut qmp = vm.qmp()?;
            stop_and_verify(&mut qmp)?;
            let offset = log
                .as_ref()
                .map(|path| {
                    fs::metadata(path)
                        .map(|metadata| metadata.len())
                        .map_err(|error| {
                            format!("measure final TCG trace {}: {error}", path.display())
                        })
                })
                .transpose()?
                .unwrap_or(0);
            final_trace_offsets.push(offset);
        }
    }
    let ticks_per_second = host["clock_ticks_per_second"].as_u64().unwrap_or(100);
    let mut guests = Vec::new();
    for index in 0..count {
        let vm = &vms[index as usize];
        let disk = out.join(format!("baseline-{index:03}.qcow2"));
        let translation = if let Some(path) = &translation_logs[index as usize] {
            let trace = fs::read_to_string(path)
                .map_err(|e| format!("read TCG trace {}: {e}", path.display()))?;
            let summary = crate::metrics::parse_tcg_translation_summary(&trace)?;
            let ready_offset = ready_trace_offsets
                .as_ref()
                .and_then(|offsets| offsets.get(index as usize))
                .copied()
                .ok_or("missing TCG trace offset at guest readiness")?;
            let idle_offset = idle_trace_offsets
                .as_ref()
                .and_then(|offsets| offsets.get(index as usize))
                .copied()
                .ok_or("missing TCG trace offset before active workload")?;
            let final_offset = final_trace_offsets
                .get(index as usize)
                .copied()
                .ok_or("missing final TCG trace offset")?;
            json!({
                "event": "translate_block",
                "events_through_final_freeze": summary.translation_block_events,
                "unique_guest_pc_count": summary.unique_guest_pc_count,
                "events_at_guest_pcs_seen_multiple_times": summary.events_at_guest_pcs_seen_multiple_times,
                "guest_pcs_seen_multiple_times": summary.guest_pcs_seen_multiple_times,
                "max_events_for_one_guest_pc": summary.max_events_for_one_guest_pc,
                "guest_pc_metric_interpretation": summary.interpretation,
                "generated_block_events": summary.generated_block_events,
                "generated_host_code_bytes_cumulative": summary.generated_host_code_bytes_cumulative,
                "translation_start_completion_event_count_delta": summary.translation_start_completion_event_count_delta,
                "generated_code_metric_interpretation": if summary.generated_block_events.is_some() { "cumulative generated TB bytes; not current code-cache residency" } else { "not available: QEMU does not expose translate_block_complete" },
                "generated_translation_wall_duration_us_cumulative": summary.generated_translation_wall_duration_us_cumulative,
                "generated_translation_wall_duration_us_max": summary.generated_translation_wall_duration_us_max,
                "translation_duration_metric_interpretation": if summary.generated_translation_wall_duration_us_cumulative.is_some() { "monotonic wall duration around setjmp_gen_code; includes scheduling delay and is not thread CPU time" } else { "not available: QEMU does not expose duration_us" },
                "phase_boundaries": {
                    "ready_freeze_log_byte": ready_offset,
                    "pre_active_freeze_log_byte": idle_offset,
                    "final_freeze_log_byte": final_offset,
                    "interpretation": "boundaries are trace-file offsets observed while QMP reports the guest paused; each boundary briefly pauses and resumes traced VMs"
                },
                "phases": {
                    "boot_through_ready_freeze": summarize_translation_phase(&trace, 0, ready_offset)?,
                    "post_ready_idle_until_pre_active_freeze": summarize_translation_phase(&trace, ready_offset, idle_offset)?,
                    "active_workload_until_final_freeze": summarize_translation_phase(&trace, idle_offset, final_offset)?
                },
                "log_path": path,
                "log_bytes": fs::metadata(path).map_err(|e| e.to_string())?.len(),
            })
        } else {
            json!({"status":"not-instrumented"})
        };
        guests.push(json!({"index":index,"pid":vm.pid(),"qemu_command":vm.command,"memory":measure(vm.pid())?,"idle_cpu_ticks":idle_cpu_ticks[index as usize],"active_cpu_ticks":active_cpu_ticks[index as usize],"active_cpu_percent_one_core":cpu_ticks_as_percent_of_one_core(active_cpu_ticks[index as usize], active_cpu_sample_interval_ms, ticks_per_second),"disk_allocated_bytes":allocated_bytes(&disk)?,"tcg_translation_trace":translation}));
    }
    let result = json!({"experiment":"baseline","accelerator":if tcg { "tcg" } else { "kvm" },"guest_cpu_model":if tcg { "max" } else { "host" },"host":host,"count":count,"guest_ram_mib":RAM_MIB,"host_mem_available_kib_at_start":available_kib,"time_to_ready_ms":elapsed,"idle_cpu_sample_interval_ms":idle_cpu_sample_interval_ms,"active_cpu_sample_interval_ms":active_cpu_sample_interval_ms,"active_cpu_workload":{"guest_command":format!("cpu-burn {ACTIVE_CPU_BURN_MS}"),"ticks_per_second":ticks_per_second},"cpu_ticks_units":"raw /proc/<pid>/stat scheduler ticks","translation_metrics":if trace_translations && trace_codegen_complete { "translate_block events, guest-PC reuse, cumulative generated TB bytes, and monotonic wall duration around setjmp_gen_code; per-phase summaries split boot/readiness, idle, and active workload; generated bytes are not current cache residency, duration is not thread CPU time" } else if trace_translations { "translate_block events and guest-PC reuse split by boot/readiness, idle, and active workload; generated code size and translation duration unavailable" } else { "not instrumented in this baseline" },"translation_trace_perturbs_timing":trace_translations,"translation_trace_phase_boundary_perturbs_timing":if trace_translations { "each traced guest is briefly paused and resumed at READY and before the active workload; phase boundaries are QMP-frozen log offsets, not exact guest instruction boundaries" } else { "not applicable: translation tracing is disabled" },"vms":guests});
    fs::write(
        out.join("baseline.json"),
        serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    println!("{}", out.join("baseline.json").display());
    Ok(())
}
