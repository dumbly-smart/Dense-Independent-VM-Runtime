use crate::metrics::parse_smaps_for_path;
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

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
    if !(1..=4).contains(&schema) {
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
    let parent_dir = if schema == 4 {
        let manifest_dir = manifest_path
            .parent()
            .ok_or("template manifest has no parent directory")?;
        let manifest_dir = if manifest_dir.is_absolute() {
            manifest_dir.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|error| error.to_string())?
                .join(manifest_dir)
        };
        manifest_dir.join(parent_dir).to_string_lossy().into_owned()
    } else {
        parent_dir.to_string()
    };
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

fn sha256_file(path: &Path) -> Result<String, String> {
    let output = command_output(Command::new("sha256sum").arg("--").arg(path))?;
    output
        .split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| format!("sha256sum returned no digest for {}", path.display()))
}

fn sha256_bytes(bytes: &[u8]) -> Result<String, String> {
    let mut child = Command::new("sha256sum")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("spawn sha256sum: {error}"))?;
    child
        .stdin
        .take()
        .ok_or("sha256sum stdin was unavailable")?
        .write_all(bytes)
        .map_err(|error| format!("write to sha256sum: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("wait for sha256sum: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "sha256sum failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| error.to_string())?
        .split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| "sha256sum returned no digest for input bytes".to_string())
}

pub fn validate_template_generation(manifest_path: &Path) -> Result<Value, String> {
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
    if !(1..=4).contains(&schema) {
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

    fn verify_hash(path: &Path, expected: &str, label: &str) -> Result<(), String> {
        if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("template manifest {label} is not a SHA-256 digest"));
        }
        if !path.is_file() {
            return Err(format!("template artifact is missing: {}", path.display()));
        }
        let actual = sha256_file(path)?;
        if actual != expected {
            return Err(format!("template artifact hash mismatch: {label}"));
        }
        Ok(())
    }

    fn local_path(manifest_path: &Path, value: &Value, section: &str) -> Result<PathBuf, String> {
        let filename = string_field(value, "path", section)?;
        let relative = Path::new(filename);
        if relative.components().count() != 1
            || !matches!(relative.components().next(), Some(Component::Normal(_)))
        {
            return Err(format!(
                "template artifact path must be a local filename: {filename}"
            ));
        }
        Ok(manifest_path
            .parent()
            .ok_or("template manifest has no parent directory")?
            .join(relative))
    }

    let parent = field(&manifest, "parent", "root")?;
    let parent_path = Path::new(string_field(parent, "template_dir", "parent")?);
    let parent_dir = if schema == 4 {
        if parent_path.is_absolute() {
            return Err("v4 parent path must be relative".to_string());
        }
        let manifest_dir = manifest_path
            .parent()
            .ok_or("template manifest has no parent directory")?;
        let manifest_dir = if manifest_dir.is_absolute() {
            manifest_dir.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|error| error.to_string())?
                .join(manifest_dir)
        };
        manifest_dir.join(parent_path)
    } else if parent_path.is_absolute() {
        parent_path.to_path_buf()
    } else {
        return Err("template parent path must be absolute".to_string());
    };
    let mut checked = 0;
    let parent_disk = if schema == 3 || schema == 4 {
        let artifacts = field(parent, "artifacts", "parent")?;
        let mut disk_filename = None;
        for key in ["result", "ram", "machine_state", "disk"] {
            let artifact = field(artifacts, key, "parent.artifacts")?;
            let filename = string_field(artifact, "path", &format!("parent.artifacts.{key}"))?;
            let relative = Path::new(filename);
            if relative.components().count() != 1
                || !matches!(relative.components().next(), Some(Component::Normal(_)))
            {
                return Err(format!(
                    "parent artifact path must be a local filename: {filename}"
                ));
            }
            let expected = string_field(artifact, "sha256", &format!("parent.artifacts.{key}"))?;
            verify_hash(
                &parent_dir.join(relative),
                expected,
                &format!("parent.artifacts.{key}"),
            )?;
            if key == "disk" {
                disk_filename = Some(filename.to_string());
            }
            checked += 1;
        }
        disk_filename.ok_or("template manifest parent disk artifact is missing")?
    } else {
        let parent_hashes = [
            ("result.json", "result_sha256"),
            ("memory.img", "ram_sha256"),
            ("machine-state", "machine_state_sha256"),
            ("base.qcow2", "base_disk_sha256"),
        ];
        for (filename, key) in parent_hashes {
            let Some(expected) = parent.get(key).and_then(Value::as_str) else {
                if schema == 1 && (key == "ram_sha256" || key == "machine_state_sha256") {
                    continue;
                }
                return Err(format!("template manifest missing parent.{key}"));
            };
            verify_hash(
                &parent_dir.join(filename),
                expected,
                &format!("parent.{key}"),
            )?;
            checked += 1;
        }
        "base.qcow2".to_string()
    };

    let ram = field(&manifest, "ram", "root")?;
    let machine_state = field(&manifest, "machine_state", "root")?;
    let disk = field(&manifest, "disk", "root")?;
    let ram_path = local_path(manifest_path, ram, "ram")?;
    let state_path = local_path(manifest_path, machine_state, "machine_state")?;
    let disk_path = local_path(manifest_path, disk, "disk")?;
    let ram_hash = string_field(ram, "sha256", "ram")?;
    let state_hash = string_field(machine_state, "sha256", "machine_state")?;
    let disk_hash = string_field(disk, "sha256", "disk")?;
    for (path, hash, label) in [
        (&ram_path, ram_hash, "ram"),
        (&state_path, state_hash, "machine_state"),
        (&disk_path, disk_hash, "disk"),
    ] {
        verify_hash(path, hash, label)?;
        checked += 1;
    }

    let backing_path = Path::new(string_field(disk, "backing_path", "disk")?);
    if schema == 4 && backing_path.is_absolute() {
        return Err("v4 disk backing path must be relative".to_string());
    }
    let backing_path = if schema == 4 && !backing_path.is_absolute() {
        manifest_path
            .parent()
            .ok_or("template manifest has no parent directory")?
            .join(backing_path)
    } else {
        backing_path.to_path_buf()
    };
    let expected_base = parent_dir.join(&parent_disk);
    if backing_path != expected_base {
        return Err("template disk backing path does not match parent base image".to_string());
    }
    let backing_hash = string_field(disk, "backing_sha256", "disk")?;
    let expected_backing_hash = if schema == 3 || schema == 4 {
        parent["artifacts"]["disk"]["sha256"].as_str()
    } else {
        parent.get("base_disk_sha256").and_then(Value::as_str)
    };
    if Some(backing_hash) != expected_backing_hash {
        return Err("template disk backing hash does not match parent base hash".to_string());
    }

    let generation_id = string_field(&manifest, "generation_id", "root")?;
    if schema == 2 || schema == 3 || schema == 4 {
        let hashes = [ram_hash, state_hash, disk_hash];
        let encoded = serde_json::to_vec(&hashes).map_err(|error| error.to_string())?;
        let expected_id = format!("sha256:{}", sha256_bytes(&encoded)?);
        if generation_id != expected_id {
            return Err("template generation ID does not match child artifact hashes".to_string());
        }
    }

    Ok(json!({
        "status": "verified",
        "integrity_verified": true,
        "schema_version": schema,
        "generation_id": generation_id,
        "checked_artifacts": checked,
    }))
}

fn collect_generation_manifests(
    directory: &Path,
    manifests: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("read template directory {}: {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("read template directory entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("inspect {}: {error}", entry.path().display()))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_generation_manifests(&entry.path(), manifests)?;
        } else if file_type.is_file() && entry.file_name() == "generation.json" {
            manifests.push(entry.path());
        }
    }
    Ok(())
}

pub fn template_tree(root: &Path) -> Result<Value, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("open template catalog {}: {error}", root.display()))?;
    if !root.is_dir() {
        return Err(format!(
            "template catalog root is not a directory: {}",
            root.display()
        ));
    }
    let mut manifests = Vec::new();
    collect_generation_manifests(&root, &mut manifests)?;
    manifests.sort();

    let mut directories = std::collections::HashMap::new();
    let mut nodes = Vec::new();
    for manifest_path in manifests {
        let manifest_path = manifest_path
            .canonicalize()
            .map_err(|error| format!("resolve manifest {}: {error}", manifest_path.display()))?;
        let metadata = template_generation(&manifest_path)?;
        let directory = manifest_path
            .parent()
            .ok_or("template manifest has no containing directory")?;
        let generation_id = metadata["generation_id"]
            .as_str()
            .ok_or("template manifest has no generation ID")?;
        let parent_directory = metadata["parent_template_dir"]
            .as_str()
            .ok_or("template manifest has no parent directory")?;
        directories.insert(directory.to_path_buf(), generation_id.to_string());
        nodes.push((
            directory.to_path_buf(),
            manifest_path,
            generation_id.to_string(),
            PathBuf::from(parent_directory),
            metadata["schema_version"].clone(),
        ));
    }

    let generations = nodes
        .into_iter()
        .map(
            |(directory, manifest, generation_id, parent_directory, schema)| {
                let resolved_parent = parent_directory.canonicalize().unwrap_or(parent_directory);
                json!({
                    "generation_id": generation_id,
                    "schema_version": schema,
                    "manifest_path": manifest,
                    "directory": directory,
                    "parent_directory": resolved_parent,
                    "parent_generation_id": directories.get(&resolved_parent),
                    "integrity_verified": false,
                })
            },
        )
        .collect::<Vec<_>>();

    Ok(json!({
        "catalog_root": root,
        "integrity_verified": false,
        "generations": generations,
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
    use super::{template_generation, template_tree, validate_template_generation};
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
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

    fn digest(path: &std::path::Path) -> String {
        let output = Command::new("sha256sum").arg(path).output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_string()
    }

    #[test]
    fn verifies_generation_artifacts_and_rejects_modified_ram() {
        let (root, _) = fixture("{}");
        let parent = root.join("parent");
        let child = root.join("child");
        fs::create_dir_all(&parent).unwrap();
        fs::create_dir_all(&child).unwrap();
        for (path, contents) in [
            (parent.join("result.json"), "parent result"),
            (parent.join("memory.img"), "parent ram"),
            (parent.join("machine-state"), "parent state"),
            (parent.join("base.qcow2"), "base disk"),
            (child.join("child-memory.img"), "child ram"),
            (child.join("child-machine-state"), "child state"),
            (child.join("parent.qcow2"), "parent overlay"),
        ] {
            fs::write(path, contents).unwrap();
        }
        let source_hashes = [
            digest(&parent.join("result.json")),
            digest(&parent.join("memory.img")),
            digest(&parent.join("machine-state")),
            digest(&parent.join("base.qcow2")),
        ];
        let child_hashes = [
            digest(&child.join("child-memory.img")),
            digest(&child.join("child-machine-state")),
            digest(&child.join("parent.qcow2")),
        ];
        let id_input = root.join("generation-id-input.json");
        fs::write(&id_input, serde_json::to_vec(&child_hashes).unwrap()).unwrap();
        let generation_id = format!("sha256:{}", digest(&id_input));
        let manifest_path = child.join("generation.json");
        fs::write(
            &manifest_path,
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 3,
                "generation_id": generation_id,
                "parent": {
                    "template_dir": parent,
                    "artifacts": {
                        "result": {"path": "result.json", "sha256": source_hashes[0]},
                        "ram": {"path": "memory.img", "sha256": source_hashes[1]},
                        "machine_state": {"path": "machine-state", "sha256": source_hashes[2]},
                        "disk": {"path": "base.qcow2", "sha256": source_hashes[3]},
                    },
                },
                "ram": {"path": "child-memory.img", "sha256": child_hashes[0], "logical_size_bytes": 12},
                "machine_state": {"path": "child-machine-state", "sha256": child_hashes[1]},
                "disk": {
                    "path": "parent.qcow2",
                    "sha256": child_hashes[2],
                    "backing_path": parent.join("base.qcow2"),
                    "backing_sha256": source_hashes[3],
                },
            }))
            .unwrap(),
        )
        .unwrap();

        let report = validate_template_generation(&manifest_path).unwrap();
        assert_eq!(report["status"], "verified");
        assert_eq!(report["checked_artifacts"], 7);

        fs::write(child.join("child-memory.img"), "modified child ram").unwrap();
        assert!(
            validate_template_generation(&manifest_path)
                .unwrap_err()
                .contains("hash mismatch")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validates_v4_generation_after_relocating_the_template_tree() {
        let (root, _) = fixture("{}");
        let parent = root.join("parent");
        let child = root.join("child");
        fs::create_dir_all(&parent).unwrap();
        fs::create_dir_all(&child).unwrap();
        for (path, contents) in [
            (parent.join("result.json"), "parent result"),
            (parent.join("memory.img"), "parent ram"),
            (parent.join("machine-state"), "parent state"),
            (parent.join("base.qcow2"), "base disk"),
            (child.join("child-memory.img"), "child ram"),
            (child.join("child-machine-state"), "child state"),
            (child.join("parent.qcow2"), "parent overlay"),
        ] {
            fs::write(path, contents).unwrap();
        }
        let parent_hashes = [
            digest(&parent.join("result.json")),
            digest(&parent.join("memory.img")),
            digest(&parent.join("machine-state")),
            digest(&parent.join("base.qcow2")),
        ];
        let child_hashes = [
            digest(&child.join("child-memory.img")),
            digest(&child.join("child-machine-state")),
            digest(&child.join("parent.qcow2")),
        ];
        let id_input = root.join("generation-id-input.json");
        fs::write(&id_input, serde_json::to_vec(&child_hashes).unwrap()).unwrap();
        let generation_id = format!("sha256:{}", digest(&id_input));
        let manifest_path = child.join("generation.json");
        fs::write(
            &manifest_path,
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 4,
                "generation_id": generation_id,
                "parent": {
                    "template_dir": "../parent",
                    "artifacts": {
                        "result": {"path": "result.json", "sha256": parent_hashes[0]},
                        "ram": {"path": "memory.img", "sha256": parent_hashes[1]},
                        "machine_state": {"path": "machine-state", "sha256": parent_hashes[2]},
                        "disk": {"path": "base.qcow2", "sha256": parent_hashes[3]},
                    },
                },
                "ram": {"path": "child-memory.img", "sha256": child_hashes[0], "logical_size_bytes": 12},
                "machine_state": {"path": "child-machine-state", "sha256": child_hashes[1]},
                "disk": {
                    "path": "parent.qcow2",
                    "sha256": child_hashes[2],
                    "backing_path": "../parent/base.qcow2",
                    "backing_sha256": parent_hashes[3],
                },
            }))
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            validate_template_generation(&manifest_path).unwrap()["status"],
            "verified"
        );
        let original = fs::read(&manifest_path).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value["parent"]["template_dir"] = serde_json::json!(parent.to_string_lossy());
        fs::write(&manifest_path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(
            validate_template_generation(&manifest_path)
                .unwrap_err()
                .contains("v4 parent path must be relative")
        );
        value = serde_json::from_slice(&original).unwrap();
        value["disk"]["backing_path"] = serde_json::json!(parent.join("base.qcow2"));
        fs::write(&manifest_path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(
            validate_template_generation(&manifest_path)
                .unwrap_err()
                .contains("v4 disk backing path must be relative")
        );
        fs::write(&manifest_path, original).unwrap();
        let relocated = root.with_file_name(format!(
            "{}-relocated",
            root.file_name().unwrap().to_string_lossy()
        ));
        fs::rename(&root, &relocated).unwrap();
        assert_eq!(
            validate_template_generation(&relocated.join("child/generation.json")).unwrap()["status"],
            "verified"
        );
        fs::remove_dir_all(relocated).unwrap();
    }

    #[test]
    fn catalog_resolves_parent_links_between_saved_generations() {
        let (root, _) = fixture("{}");
        fs::remove_file(root.join("generation.json")).unwrap();
        let first = root.join("generation-a");
        let second = root.join("generation-b");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let write_generation = |directory: &PathBuf, id: &str, parent_dir: &str| {
            fs::write(
                directory.join("generation.json"),
                serde_json::to_vec(&serde_json::json!({
                    "schema_version": 2,
                    "generation_id": id,
                    "parent": {"template_dir": parent_dir},
                    "ram": {"path": "ram.img", "sha256": "ram-hash", "logical_size_bytes": 12},
                    "machine_state": {"path": "state", "sha256": "state-hash"},
                    "disk": {"path": "disk.qcow2", "sha256": "disk-hash", "backing_path": "/base.qcow2"}
                }))
                .unwrap(),
            )
            .unwrap();
        };
        write_generation(&first, "sha256:first", "/external/root");
        write_generation(&second, "sha256:second", first.to_str().unwrap());

        let tree = template_tree(&root).unwrap();

        assert_eq!(tree["integrity_verified"], false);
        assert_eq!(tree["generations"].as_array().unwrap().len(), 2);
        assert_eq!(tree["generations"][0]["generation_id"], "sha256:first");
        assert_eq!(tree["generations"][1]["generation_id"], "sha256:second");
        assert_eq!(
            tree["generations"][1]["parent_generation_id"],
            "sha256:first"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
