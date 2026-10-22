use crate::metrics::parse_smaps_for_path;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::Command;

fn command_output(command: &mut Command) -> Result<String, String> {
    let output = command
        .output()
        .map_err(|e| format!("spawn command: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "command exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

pub fn disk(image: &Path) -> Result<Value, String> {
    let output = command_output(
        Command::new("qemu-img")
            .args(["info", "--output=json", "--backing-chain"])
            .arg(image),
    )?;
    serde_json::from_str(&output).map_err(|e| format!("invalid qemu-img JSON: {e}"))
}

pub fn memory_map(pid: u32, path: &Path) -> Result<Value, String> {
    let smaps = fs::read_to_string(format!("/proc/{pid}/smaps"))
        .map_err(|e| format!("read smaps for {pid}: {e}"))?;
    let totals = parse_smaps_for_path(&smaps, path.to_str().ok_or("RAM path is not UTF-8")?)?;
    serde_json::to_value(totals).map_err(|e| e.to_string())
}

pub fn ksm_status() -> Result<Value, String> {
    let root = Path::new("/sys/kernel/mm/ksm");
    let names = [
        "run",
        "pages_shared",
        "pages_sharing",
        "pages_unshared",
        "pages_volatile",
        "full_scans",
    ];
    let mut result = serde_json::Map::new();
    for name in names {
        let value =
            fs::read_to_string(root.join(name)).map_err(|e| format!("read KSM {name}: {e}"))?;
        result.insert(
            name.to_string(),
            json!(
                value
                    .trim()
                    .parse::<u64>()
                    .map_err(|e| format!("parse KSM {name}: {e}"))?
            ),
        );
    }
    Ok(Value::Object(result))
}
