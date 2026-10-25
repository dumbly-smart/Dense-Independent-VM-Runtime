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

pub fn template_generation(manifest_path: &Path) -> Result<Value, String> {
    let contents = fs::read_to_string(manifest_path).map_err(|error| {
        format!(
            "read template manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    let manifest: Value = serde_json::from_str(&contents)
        .map_err(|error| format!("invalid template manifest JSON: {error}"))?;
    let schema = manifest
        .get("schema_version")
        .and_then(Value::as_u64)
        .ok_or("template manifest has no numeric schema_version")?;
    if schema != 1 && schema != 2 {
        return Err(format!("unsupported template manifest schema {schema}"));
    }

    fn field<'a>(value: &'a Value, key: &str, section: &str) -> Result<&'a Value, String> {
        value
            .get(key)
            .ok_or_else(|| format!("template manifest missing {section}.{key}"))
    }

    fn string_field<'a>(value: &'a Value, key: &str, section: &str) -> Result<&'a str, String> {
        field(value, key, section)?
            .as_str()
            .ok_or_else(|| format!("template manifest {section}.{key} must be a string"))
    }

    let generation_id = string_field(&manifest, "generation_id", "root")?;
    let parent = field(&manifest, "parent", "root")?;
    let parent_dir = string_field(parent, "template_dir", "parent")?;
    let ram = field(&manifest, "ram", "root")?;
    let machine_state = field(&manifest, "machine_state", "root")?;
    let disk = field(&manifest, "disk", "root")?;
    let logical_size_bytes = field(ram, "logical_size_bytes", "ram")?
        .as_u64()
        .ok_or("template manifest ram.logical_size_bytes must be a non-negative integer")?;

    Ok(json!({
        "inspection": "metadata-only",
        "integrity_verified": false,
        "schema_version": schema,
        "generation_id": generation_id,
        "parent_template_dir": parent_dir,
        "ram": {
            "path": string_field(ram, "path", "ram")?,
            "sha256": string_field(ram, "sha256", "ram")?,
            "logical_size_bytes": logical_size_bytes,
        },
        "machine_state": {
            "path": string_field(machine_state, "path", "machine_state")?,
            "sha256": string_field(machine_state, "sha256", "machine_state")?,
        },
        "disk": {
            "path": string_field(disk, "path", "disk")?,
            "sha256": string_field(disk, "sha256", "disk")?,
            "backing_path": string_field(disk, "backing_path", "disk")?,
        },
    }))
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

#[cfg(test)]
mod template_tests {
    use super::template_generation;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(contents: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "densevm-template-inspect-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let manifest = root.join("generation.json");
        fs::write(&manifest, contents).unwrap();
        (root, manifest)
    }

    #[test]
    fn reports_generation_metadata_without_claiming_hash_verification() {
        let (root, manifest) = fixture(
            r#"{
                "schema_version": 2,
                "generation_id": "sha256:abc",
                "parent": {"template_dir": "/templates/root"},
                "ram": {"path": "child-memory.img", "sha256": "ram-hash", "logical_size_bytes": 268435456},
                "machine_state": {"path": "child-machine-state", "sha256": "state-hash"},
                "disk": {"path": "parent.qcow2", "sha256": "disk-hash", "backing_path": "/templates/root/base.qcow2"}
            }"#,
        );

        let report = template_generation(&manifest).unwrap();

        assert_eq!(report["integrity_verified"], false);
        assert_eq!(report["generation_id"], "sha256:abc");
        assert_eq!(report["ram"]["logical_size_bytes"], 268435456_u64);
        assert_eq!(report["disk"]["backing_path"], "/templates/root/base.qcow2");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unknown_generation_manifest_schema() {
        let (root, manifest) = fixture(r#"{"schema_version":99}"#);
        assert!(
            template_generation(&manifest)
                .unwrap_err()
                .contains("unsupported template manifest schema")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
