//! Read-only host discovery. The QMP probe starts a stopped, diskless QEMU.
use crate::qmp::Qmp;
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const MACHINE: &str = "pc-q35-11.1";

fn kvm_capability(open: Result<(), String>) -> Value {
    match open {
        Ok(()) => {
            json!({"accessible":true,"qualification":"device opened; live guest probe must confirm KVM enabled"})
        }
        Err(error) => json!({"accessible":false,"error":error}),
    }
}

fn ksm_capability(status: Result<Value, String>) -> Value {
    match status {
        Ok(counters) => json!({"available":true,"counters":counters}),
        Err(error) => json!({"available":false,"error":error}),
    }
}

pub fn validate_template_support(migration: &Value, properties: &Value) -> Result<(), String> {
    if !migration.as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["capability"] == "x-ignore-shared")
    }) {
        return Err("QEMU lacks required migration capability x-ignore-shared".into());
    }
    for name in ["share", "readonly", "rom", "mem-path"] {
        if !properties
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["name"] == name))
        {
            return Err(format!("QEMU memory-backend-file lacks {name}"));
        }
    }
    Ok(())
}

fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let result = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if !result.status.success() {
        return Err(format!(
            "{program}: {}: {}",
            result.status,
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&result.stdout).trim().to_owned())
}

fn command_value(program: &str, args: &[&str]) -> Value {
    match output(program, args) {
        Ok(value) => json!({"available":true,"output":value}),
        Err(error) => json!({"available":false,"error":error}),
    }
}

struct Diagnostic {
    child: Child,
    directory: PathBuf,
}

impl Drop for Diagnostic {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(self.directory.join("qmp"));
        let _ = fs::remove_dir(&self.directory);
    }
}

fn qmp_report() -> Result<Value, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("densevm-doctor-{}-{stamp}", std::process::id()));
    fs::create_dir(&directory).map_err(|e| e.to_string())?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    let socket = directory.join("qmp");
    let child = Command::new("qemu-system-x86_64")
        .args([
            "-machine",
            "none,accel=tcg",
            "-S",
            "-nodefaults",
            "-display",
            "none",
            "-qmp",
        ])
        .arg(format!("unix:{},server=on,wait=off", socket.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let child = match child {
        Ok(child) => child,
        Err(e) => {
            let _ = fs::remove_dir(&directory);
            return Err(format!("start diagnostic QEMU: {e}"));
        }
    };
    let mut diagnostic = Diagnostic { child, directory };
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut qmp = loop {
        match Qmp::connect(&socket, Duration::from_secs(5)) {
            Ok(qmp) => break qmp,
            Err(error) => {
                if diagnostic
                    .child
                    .try_wait()
                    .map_err(|e| e.to_string())?
                    .is_some()
                    || Instant::now() >= deadline
                {
                    return Err(error);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let migration = qmp.execute("query-migrate-capabilities", None)?;
    let properties = qmp.execute(
        "qom-list-properties",
        Some(json!({"typename":"memory-backend-file"})),
    )?;
    let machines = qmp.execute("query-machines", None)?;
    let machine_supported = machines
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["name"] == MACHINE));
    let support = validate_template_support(&migration, &properties);
    Ok(json!({
        "available":true,
        "diagnostic_accelerator":"tcg (stopped, no guest)",
        "version":qmp.execute("query-version", None)?,
        "migration_capabilities":migration,
        "memory_backend_properties":properties,
        "required_machine":MACHINE,
        "machine_supported":machine_supported,
        "template_supported":support.is_ok() && machine_supported,
        "template_error":support.err(),
    }))
}

pub fn host_report() -> Value {
    let kvm = kvm_capability(
        OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/kvm")
            .map(drop)
            .map_err(|error| error.to_string()),
    );
    json!({
        "kernel_version":output("uname", &["-r"]).ok(),
        "qemu":command_value("qemu-system-x86_64", &["--version"]),
        "qemu_img":command_value("qemu-img", &["--version"]),
        "page_size_bytes":output("getconf", &["PAGESIZE"]).ok().and_then(|s| s.parse::<u64>().ok()),
        "clock_ticks_per_second":output("getconf", &["CLK_TCK"]).ok().and_then(|s| s.parse::<u64>().ok()),
        "kvm":kvm,
        "qmp":qmp_report().unwrap_or_else(|error| json!({"available":false,"error":error})),
        "ksm":ksm_capability(crate::inspect::ksm_status()),
        "kernel_memory_accounting":"UNKNOWN: process PSS excludes KVM metadata and host page tables",
    })
}

#[cfg(test)]
mod tests {
    use super::{ksm_capability, kvm_capability};
    use serde_json::json;

    #[test]
    fn unavailable_kvm_and_ksm_are_reported_as_unsupported() {
        let kvm = kvm_capability(Err("No such file or directory".into()));
        let ksm = ksm_capability(Err("read KSM pages_shared: not found".into()));
        assert_eq!(kvm["accessible"], false);
        assert_eq!(ksm["available"], false);
        assert!(ksm["counters"].is_null());
    }

    #[test]
    fn available_ksm_requires_a_complete_counter_result() {
        let ksm = ksm_capability(Ok(json!({
            "run":0,
            "pages_shared":0,
            "pages_sharing":0,
            "pages_unshared":0,
            "pages_volatile":0,
            "full_scans":0
        })));
        assert_eq!(ksm["available"], true);
        assert_eq!(ksm["counters"]["pages_shared"], 0);
    }
}

pub fn require_template_support(report: &Value) -> Result<(), String> {
    if report["kvm"]["accessible"] != true {
        return Err(format!("/dev/kvm is not accessible: {}", report["kvm"]));
    }
    if report["qmp"]["template_supported"] != true {
        return Err(format!(
            "QEMU does not support the pinned template configuration: {}",
            report["qmp"]
        ));
    }
    Ok(())
}
