use crate::metrics::{parse_smaps_for_path, ram_mapping_report};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Deserialize, Serialize)]
struct TemplateGcJournal {
    schema_version: u32,
    generation_id: String,
    generation_dir: PathBuf,
    manifest_sha256: String,
    artifacts: Vec<TemplateGcArtifact>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct TemplateGcArtifact {
    kind: String,
    name: String,
    sha256: String,
}

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

fn clone_disk_path(id: &str, state_dir: &Path, recorded_path: &Path) -> Result<PathBuf, String> {
    let id_path = Path::new(id);
    if id_path.components().count() != 1
        || !matches!(id_path.components().next(), Some(Component::Normal(_)))
    {
        return Err("clone ID is not a local path component".into());
    }
    if !recorded_path.is_absolute() {
        return Err("recorded clone disk path is not absolute".into());
    }
    let state_root = state_dir
        .canonicalize()
        .map_err(|error| format!("resolve clone state directory: {error}"))?;
    let instance = state_root
        .join(id)
        .canonicalize()
        .map_err(|error| format!("resolve clone instance directory: {error}"))?;
    if instance.parent() != Some(state_root.as_path()) {
        return Err("clone instance directory escapes the state directory".into());
    }
    let expected = instance.join("disk.qcow2");
    let metadata = fs::symlink_metadata(recorded_path)
        .map_err(|error| format!("inspect recorded clone disk: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err("recorded clone disk is not a regular file".into());
    }
    let recorded = recorded_path
        .canonicalize()
        .map_err(|error| format!("resolve recorded clone disk: {error}"))?;
    if recorded != expected {
        return Err("recorded clone disk path is outside its private instance directory".into());
    }
    Ok(recorded)
}

pub fn inspect_clone_disk(id: &str, state_dir: &Path) -> Result<Value, String> {
    let registry = crate::lifecycle::CloneRegistry::open_read_only(state_dir)?;
    let record = registry.get(id)?;
    let path = clone_disk_path(id, state_dir, &record.disk_overlay)?;
    let disk_info = disk(&path)?;
    Ok(json!({
        "clone": {
            "id":record.id,
            "generation_id":record.generation_id,
            "state":record.state,
            "disk_overlay":path,
        },
        "disk":disk_info,
        "measurement_limit":"qemu-img metadata sampled while the guest is running may race concurrent guest writes",
    }))
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

fn sync_directory(path: &Path) -> Result<(), String> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("sync directory {}: {error}", path.display()))
}

fn template_gc_journals(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut journals = Vec::new();
    for entry in fs::read_dir(root)
        .map_err(|error| format!("list template GC journals in {}: {error}", root.display()))?
    {
        let entry = entry.map_err(|error| format!("read template catalog entry: {error}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".densevm-gc-") && name.ends_with(".json") {
            let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
                format!("inspect GC journal {}: {error}", entry.path().display())
            })?;
            if !metadata.file_type().is_file() {
                return Err(format!(
                    "GC journal is not a regular file: {}",
                    entry.path().display()
                ));
            }
            journals.push(entry.path());
        }
    }
    journals.sort();
    Ok(journals)
}

fn gc_journal_path(root: &Path, generation_id: &str) -> Result<PathBuf, String> {
    let digest = generation_id
        .strip_prefix("sha256:")
        .ok_or("GC generation ID must use sha256:<64 hex digits>")?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("GC generation ID must use sha256:<64 hex digits>".into());
    }
    Ok(root.join(format!(".densevm-gc-{digest}.json")))
}

fn write_gc_journal(
    root: &Path,
    journal_path: &Path,
    journal: &TemplateGcJournal,
) -> Result<(), String> {
    let bytes =
        serde_json::to_vec(journal).map_err(|error| format!("encode GC journal: {error}"))?;
    let temporary = root.join(format!(
        ".densevm-gc-tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| format!("clock for GC journal: {error}"))?
            .as_nanos()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("create GC journal {}: {error}", temporary.display()))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("persist GC journal {}: {error}", temporary.display()))?;
    fs::rename(&temporary, journal_path)
        .map_err(|error| format!("publish GC journal {}: {error}", journal_path.display()))?;
    sync_directory(root)
}

fn read_gc_journal(root: &Path, path: &Path) -> Result<TemplateGcJournal, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect GC journal {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "GC journal is not a regular file: {}",
            path.display()
        ));
    }
    let journal: TemplateGcJournal = serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("read GC journal {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("parse GC journal {}: {error}", path.display()))?;
    if journal.schema_version != 1 || gc_journal_path(root, &journal.generation_id)? != path {
        return Err(format!(
            "unsupported or mismatched GC journal {}",
            path.display()
        ));
    }
    let directory = journal
        .generation_dir
        .canonicalize()
        .map_err(|error| format!("resolve journal generation directory: {error}"))?;
    if !directory.starts_with(root) || directory == root || directory != journal.generation_dir {
        return Err(
            "GC journal generation directory escapes the catalog or is not canonical".into(),
        );
    }
    let mut kinds = std::collections::BTreeSet::new();
    for artifact in &journal.artifacts {
        if !matches!(artifact.kind.as_str(), "disk" | "machine_state" | "ram")
            || !kinds.insert(artifact.kind.as_str())
            || artifact.name.is_empty()
            || Path::new(&artifact.name).components().count() != 1
            || !matches!(
                Path::new(&artifact.name).components().next(),
                Some(Component::Normal(_))
            )
            || artifact.sha256.len() != 64
            || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!(
                "invalid artifact entry in GC journal {}",
                path.display()
            ));
        }
    }
    if kinds.len() != 3 {
        return Err(format!(
            "GC journal {} must list disk, machine_state, and ram",
            path.display()
        ));
    }
    Ok(journal)
}

fn resume_gc_journal_locked(root: &Path, state_dir: &Path, path: &Path) -> Result<Value, String> {
    let journal = read_gc_journal(root, path)?;
    let registry = crate::lifecycle::CloneRegistry::open_read_only(state_dir)?;
    let referenced = registry.list()?.into_iter().any(|clone| {
        clone.generation_id == journal.generation_id
            && matches!(
                clone.state,
                crate::lifecycle::CloneState::Starting | crate::lifecycle::CloneState::Running
            )
    });
    if referenced {
        return Err(format!(
            "generation {} is referenced by an active clone",
            journal.generation_id
        ));
    }
    let manifest_path = journal.generation_dir.join("generation.json");
    let manifest_exists = match fs::symlink_metadata(&manifest_path) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("inspect journal manifest: {error}")),
    };
    if manifest_exists {
        let metadata = fs::symlink_metadata(&manifest_path)
            .map_err(|error| format!("inspect journal manifest: {error}"))?;
        if !metadata.file_type().is_file()
            || sha256_file(&manifest_path)? != journal.manifest_sha256
        {
            return Err("GC journal manifest is not the recorded regular file".into());
        }
        let manifest: Value = serde_json::from_slice(
            &fs::read(&manifest_path).map_err(|error| format!("read journal manifest: {error}"))?,
        )
        .map_err(|error| format!("parse journal manifest: {error}"))?;
        if manifest["generation_id"] != journal.generation_id {
            return Err("GC journal generation ID does not match its manifest".into());
        }
        for artifact in &journal.artifacts {
            if manifest[&artifact.kind]["path"].as_str() != Some(&artifact.name) {
                return Err(format!(
                    "GC journal {} path does not match manifest",
                    artifact.kind
                ));
            }
        }
    }
    let mut remaining_artifact = false;
    for artifact in &journal.artifacts {
        let artifact_path = journal.generation_dir.join(&artifact.name);
        match fs::symlink_metadata(&artifact_path) {
            Ok(metadata) => {
                remaining_artifact = true;
                if !metadata.file_type().is_file()
                    || sha256_file(&artifact_path)? != artifact.sha256
                {
                    return Err(format!(
                        "GC artifact is unsafe or changed: {}",
                        artifact_path.display()
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "inspect GC artifact {}: {error}",
                    artifact_path.display()
                ));
            }
        }
    }
    if !manifest_exists && remaining_artifact {
        return Err("GC manifest is missing while artifacts remain; refusing recovery".into());
    }
    for artifact in &journal.artifacts {
        let artifact_path = journal.generation_dir.join(&artifact.name);
        if artifact_path.exists() {
            fs::remove_file(&artifact_path).map_err(|error| {
                format!("resume GC unlink {}: {error}", artifact_path.display())
            })?;
        }
    }
    sync_directory(&journal.generation_dir)?;
    if manifest_path.exists() {
        fs::remove_file(&manifest_path).map_err(|error| format!("remove GC manifest: {error}"))?;
        sync_directory(&journal.generation_dir)?;
    }
    fs::remove_file(path)
        .map_err(|error| format!("remove completed GC journal {}: {error}", path.display()))?;
    sync_directory(root)?;
    Ok(
        json!({"generation_id":journal.generation_id,"generation_dir":journal.generation_dir,"status":"recovered"}),
    )
}

pub fn recover_template_gc(root: &Path, state_dir: &Path) -> Result<Value, String> {
    let _tree_lock = crate::template_lock::TemplateTreeLock::acquire_exclusive(root, true)?;
    let root = root
        .canonicalize()
        .map_err(|error| format!("resolve template-tree root: {error}"))?;
    let journals = template_gc_journals(&root)?;
    let mut recovered = Vec::new();
    for journal in journals {
        let record = read_gc_journal(&root, &journal)?;
        let _generation_lock = crate::template_lock::TemplateTreeLock::acquire_exclusive(
            &record.generation_dir,
            true,
        )?;
        recovered.push(resume_gc_journal_locked(&root, state_dir, &journal)?);
    }
    Ok(
        json!({"status":if recovered.is_empty() {"nothing_to_recover"} else {"recovered"},"recovered":recovered,"power_loss_durability":"not proven; recovery is designed for process interruption and uses file/directory sync calls"}),
    )
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

pub fn validate_template_tree(root: &Path) -> Result<Value, String> {
    let mut report = template_tree(root)?;
    let generations = report["generations"]
        .as_array_mut()
        .ok_or("template tree omitted its generations array")?;
    let generation_count = generations.len();
    let mut all_verified = generation_count > 0;
    let mut generation_ids = std::collections::HashSet::new();
    let mut duplicate_generation_ids = Vec::new();

    for generation in generations.iter_mut() {
        let generation_id = generation["generation_id"]
            .as_str()
            .ok_or("template tree generation omitted its ID")?
            .to_string();
        if !generation_ids.insert(generation_id.clone()) {
            duplicate_generation_ids.push(generation_id);
            all_verified = false;
        }
        let manifest_path = generation["manifest_path"]
            .as_str()
            .ok_or("template tree generation omitted its manifest path")?;
        match validate_template_generation(Path::new(manifest_path)) {
            Ok(validation) => {
                generation["integrity_verified"] = json!(true);
                generation["validation"] = validation;
            }
            Err(error) => {
                all_verified = false;
                generation["integrity_verified"] = json!(false);
                generation["validation"] = json!({
                    "status":"failed",
                    "integrity_verified":false,
                    "error":error
                });
            }
        }
    }

    report["status"] = json!(if generation_count == 0 {
        "empty"
    } else if all_verified {
        "verified"
    } else {
        "failed"
    });
    report["integrity_verified"] = json!(all_verified);
    report["validated_generation_count"] = json!(generation_count);
    report["duplicate_generation_ids"] = json!(duplicate_generation_ids);
    report["validation_scope"] = json!(
        "hashes and generation ID for each manifest-referenced child and parent artifact; parent directory links are reported separately"
    );
    Ok(report)
}

/// Produce a non-destructive mark-and-sweep plan. No generation files are removed.
pub fn plan_template_gc(
    root: &Path,
    state_dir: &Path,
    keep_ids: &[String],
) -> Result<Value, String> {
    let _tree_lock = crate::template_lock::TemplateTreeLock::acquire_exclusive(root, true)?;
    plan_template_gc_locked(root, state_dir, keep_ids)
}

fn plan_template_gc_locked(
    root: &Path,
    state_dir: &Path,
    keep_ids: &[String],
) -> Result<Value, String> {
    let pending_journals = template_gc_journals(root)?;
    if !pending_journals.is_empty() {
        return Ok(json!({
            "status":"blocked",
            "dry_run":true,
            "deletes_files":false,
            "candidate_generation_ids":[],
            "candidate_leaves":[],
            "candidate_generations":[],
            "blockers":pending_journals.iter().map(|path| format!(
                "pending GC journal {}; run template gc-recover before planning",
                path.display()
            )).collect::<Vec<_>>(),
        }));
    }
    let tree = validate_template_tree(root)?;
    let registry = crate::lifecycle::CloneRegistry::open_read_only(state_dir)?;
    let clones = registry.list()?;
    let active_clones = clones
        .iter()
        .filter(|clone| {
            matches!(
                clone.state,
                crate::lifecycle::CloneState::Starting | crate::lifecycle::CloneState::Running
            )
        })
        .map(|clone| (clone.id.clone(), clone.generation_id.clone(), clone.state))
        .collect::<Vec<_>>();
    let active_ids = active_clones
        .iter()
        .map(|(_, id, _)| id.clone())
        .collect::<Vec<_>>();
    let mut report = plan_template_gc_report(tree, keep_ids, &active_ids);
    report["active_clones"] = json!(
        active_clones
            .iter()
            .map(|(clone_id, generation_id, state)| {
                json!({"clone_id":clone_id,"generation_id":generation_id,"state":state})
            })
            .collect::<Vec<_>>()
    );
    report["state_dir"] = json!(state_dir);
    Ok(report)
}

pub fn delete_template_gc_leaf(
    root: &Path,
    state_dir: &Path,
    generation_id: &str,
    keep_ids: &[String],
) -> Result<Value, String> {
    let _tree_lock = crate::template_lock::TemplateTreeLock::acquire_exclusive(root, true)?;
    let root = root
        .canonicalize()
        .map_err(|error| format!("resolve template-tree root: {error}"))?;
    let plan = plan_template_gc_locked(&root, state_dir, keep_ids)?;
    if plan["status"] != "planned" {
        return Ok(json!({
            "status":"blocked",
            "generation_id":generation_id,
            "blockers":plan["blockers"],
            "plan":plan,
            "deletes_files":false,
        }));
    }
    let candidate = plan["candidate_generations"]
        .as_array()
        .and_then(|candidates| {
            candidates
                .iter()
                .find(|candidate| candidate["generation_id"].as_str() == Some(generation_id))
        });
    let Some(candidate) = candidate else {
        return Ok(json!({
            "status":"blocked",
            "generation_id":generation_id,
            "blockers":["generation is not an unreferenced GC candidate"],
            "deletes_files":false,
        }));
    };
    if candidate["is_leaf"] != true {
        return Ok(json!({
            "status":"blocked",
            "generation_id":generation_id,
            "blockers":["only an unreferenced leaf generation can be collected"],
            "deletes_files":false,
        }));
    }
    let directory = PathBuf::from(
        candidate["directory"]
            .as_str()
            .ok_or("GC candidate omitted its directory")?,
    )
    .canonicalize()
    .map_err(|error| format!("resolve GC candidate directory: {error}"))?;
    if !directory.starts_with(&root) {
        return Err("GC candidate directory escapes the template catalog".into());
    }
    let _generation_lock = if directory == root {
        None
    } else {
        Some(crate::template_lock::TemplateTreeLock::acquire_exclusive(
            &directory, true,
        )?)
    };

    // Recheck reachability after taking the generation lock. Clone startup
    // takes the shared form of this same lock while reading its template.
    let plan = plan_template_gc_locked(&root, state_dir, keep_ids)?;
    let candidate = plan["candidate_generations"]
        .as_array()
        .and_then(|candidates| {
            candidates.iter().find(|candidate| {
                candidate["generation_id"].as_str() == Some(generation_id)
                    && candidate["is_leaf"] == true
            })
        });
    let Some(candidate) = candidate else {
        return Ok(json!({
            "status":"blocked",
            "generation_id":generation_id,
            "blockers":["generation became protected or is no longer a leaf"],
            "deletes_files":false,
        }));
    };
    let manifest_path = PathBuf::from(
        candidate["manifest_path"]
            .as_str()
            .ok_or("GC candidate omitted its manifest path")?,
    );
    if manifest_path.parent() != Some(directory.as_path()) {
        return Err("GC manifest is not directly inside its generation directory".into());
    }
    let manifest_metadata = fs::symlink_metadata(&manifest_path)
        .map_err(|error| format!("inspect GC manifest {}: {error}", manifest_path.display()))?;
    if !manifest_metadata.file_type().is_file() {
        return Err("GC manifest is not a regular file".into());
    }
    let manifest: Value = serde_json::from_slice(
        &fs::read(&manifest_path).map_err(|error| format!("read GC manifest: {error}"))?,
    )
    .map_err(|error| format!("parse GC manifest: {error}"))?;
    let mut artifacts = Vec::new();
    for section in ["disk", "machine_state", "ram"] {
        let filename = manifest[section]["path"]
            .as_str()
            .ok_or_else(|| format!("GC manifest omitted {section}.path"))?;
        let relative = Path::new(filename);
        if relative.components().count() != 1
            || !matches!(relative.components().next(), Some(Component::Normal(_)))
        {
            return Err(format!(
                "GC artifact path is not a local filename: {filename}"
            ));
        }
        let artifact = directory.join(relative);
        let metadata = fs::symlink_metadata(&artifact)
            .map_err(|error| format!("inspect GC artifact {}: {error}", artifact.display()))?;
        if !metadata.file_type().is_file() {
            return Err(format!(
                "GC artifact is not a regular file: {}",
                artifact.display()
            ));
        }
        if !artifacts
            .iter()
            .any(|(existing, _): &(PathBuf, TemplateGcArtifact)| existing == &artifact)
        {
            artifacts.push((
                artifact,
                TemplateGcArtifact {
                    kind: section.to_string(),
                    name: filename.to_string(),
                    sha256: sha256_file(&directory.join(relative))?,
                },
            ));
        }
    }
    let manifest_sha256 = sha256_file(&manifest_path)?;
    let journal_path = gc_journal_path(&root, generation_id)?;
    let journal = TemplateGcJournal {
        schema_version: 1,
        generation_id: generation_id.to_string(),
        generation_dir: directory.clone(),
        manifest_sha256,
        artifacts: artifacts.iter().map(|(_, entry)| entry.clone()).collect(),
    };
    write_gc_journal(&root, &journal_path, &journal)?;
    let recovery = resume_gc_journal_locked(&root, state_dir, &journal_path)?;
    let mut deleted_files = artifacts
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    deleted_files.push(manifest_path.clone());
    let mut preserved_entries = fs::read_dir(&directory)
        .map_err(|error| format!("read remaining generation directory: {error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    preserved_entries.sort();
    Ok(json!({
        "status":"deleted",
        "generation_id":generation_id,
        "deleted_files":deleted_files,
        "generation_directory_preserved":true,
        "unrecognized_entries_preserved":preserved_entries,
        "journal_recovery":recovery,
        "deletes_files":true,
        "safety":"A synced journal records exact hashes before unlink. Only manifest-referenced RAM, machine-state, disk, and manifest files were removed from an unreferenced leaf. Other entries were preserved.",
    }))
}

fn plan_template_gc_report(tree: Value, keep_ids: &[String], active_ids: &[String]) -> Value {
    let mut blockers = Vec::new();
    let generations = tree["generations"].as_array().cloned().unwrap_or_default();
    let mut parents = std::collections::BTreeMap::<String, Option<String>>::new();

    if tree["status"] != "verified" || tree["integrity_verified"] != true || generations.is_empty()
    {
        blockers.push("template tree is empty or its integrity is not fully verified".to_string());
    }
    for generation in &generations {
        let Some(id) = generation["generation_id"].as_str() else {
            blockers.push("generation is missing a valid generation_id".to_string());
            continue;
        };
        if generation["integrity_verified"] != true {
            blockers.push(format!("generation {id} is not integrity-verified"));
        }
        if generation["directory"].as_str().is_none()
            || generation["manifest_path"].as_str().is_none()
        {
            blockers.push(format!("generation {id} is missing its catalog paths"));
        }
        if !generation["parent_generation_id"].is_null()
            && generation["parent_generation_id"].as_str().is_none()
        {
            blockers.push(format!(
                "generation {id} has an invalid parent generation reference"
            ));
        }
        let parent = generation["parent_generation_id"]
            .as_str()
            .map(str::to_string);
        if parents.insert(id.to_string(), parent).is_some() {
            blockers.push(format!("duplicate generation ID {id}"));
        }
    }
    for (id, parent) in &parents {
        if let Some(parent) = parent
            && !parents.contains_key(parent)
        {
            blockers.push(format!(
                "generation {id} references missing parent generation {parent}"
            ));
        }
    }
    for id in parents.keys() {
        let mut seen = std::collections::HashSet::new();
        let mut current = Some(id.as_str());
        while let Some(node) = current {
            if !seen.insert(node) {
                blockers.push(format!("parent-generation cycle includes {node}"));
                break;
            }
            current = parents.get(node).and_then(Option::as_deref);
        }
    }

    let mut roots = Vec::new();
    roots.extend(keep_ids.iter().map(|id| (id, "explicit_keep")));
    roots.extend(active_ids.iter().map(|id| (id, "active_clone")));
    for (id, _) in &roots {
        if !parents.contains_key(id.as_str()) {
            blockers.push(format!(
                "protected generation {id} is absent from the verified catalog"
            ));
        }
    }
    if !blockers.is_empty() {
        blockers.sort();
        blockers.dedup();
        return json!({
            "status":"blocked", "dry_run":true, "deletes_files":false,
            "candidate_generation_ids":[], "candidate_leaves":[], "protected_generation_ids":[],
            "candidate_generations":[],
            "blockers":blockers,
            "safety":"No candidates are reported unless the complete catalog is verified and every protected reference resolves."
        });
    }

    let mut protected = std::collections::BTreeSet::new();
    let mut reasons = std::collections::BTreeMap::<String, Vec<String>>::new();
    for (root, reason) in roots {
        let mut current = Some(root.as_str());
        let mut first = true;
        while let Some(id) = current {
            protected.insert(id.to_string());
            reasons.entry(id.to_string()).or_default().push(if first {
                reason.to_string()
            } else {
                format!("ancestor_of:{root}")
            });
            first = false;
            current = parents.get(id).and_then(Option::as_deref);
        }
    }
    let candidates = parents
        .keys()
        .filter(|id| !protected.contains(*id))
        .cloned()
        .collect::<Vec<_>>();
    let candidate_set = candidates
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    let leaves = candidates
        .iter()
        .filter(|id| {
            !parents.iter().any(|(child, parent)| {
                candidate_set.contains(child) && parent.as_deref() == Some(id.as_str())
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    let leaf_set = leaves
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    let candidate_generations = generations
        .iter()
        .filter_map(|generation| {
            let id = generation["generation_id"].as_str()?;
            if !candidate_set.contains(id) {
                return None;
            }
            Some(json!({
                "generation_id": id,
                "directory": generation["directory"],
                "manifest_path": generation["manifest_path"],
                "parent_generation_id": generation["parent_generation_id"],
                "is_leaf": leaf_set.contains(id),
            }))
        })
        .collect::<Vec<_>>();
    json!({
        "status":"planned", "dry_run":true, "deletes_files":false,
        "candidate_generation_ids":candidates, "candidate_leaves":leaves,
        "candidate_generations":candidate_generations,
        "protected_generation_ids":protected, "protection_reasons":reasons,
        "blockers":[],
        "safety":"Candidates are unreferenced by active clones and explicit keeps; this report does not delete or detach files."
    })
}

pub fn memory_map(pid: u32, path: &Path) -> Result<Value, String> {
    let smaps = fs::read_to_string(format!("/proc/{pid}/smaps"))
        .map_err(|e| format!("read smaps for {pid}: {e}"))?;
    let totals = parse_smaps_for_path(&smaps, path.to_str().ok_or("RAM path is not UTF-8")?)?;
    let page_size = command_output(Command::new("getconf").arg("PAGESIZE"))?
        .trim()
        .parse::<u64>()
        .map_err(|error| format!("invalid host page size: {error}"))?;
    ram_mapping_report(&totals, page_size)
}

fn qemu_ram_file_from_args(args: &[&[u8]]) -> Result<PathBuf, String> {
    let mut paths = Vec::new();
    for pair in args.windows(2) {
        if pair[0] != b"-object" {
            continue;
        }
        let object = std::str::from_utf8(pair[1])
            .map_err(|error| format!("QEMU object argument is not UTF-8: {error}"))?;
        if !object.starts_with("memory-backend-file,id=ram,") {
            continue;
        }
        let path = object
            .split(',')
            .find_map(|option| option.strip_prefix("mem-path="))
            .ok_or("QEMU RAM backend is missing mem-path")?;
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("QEMU RAM backing path is not absolute".into());
        }
        paths.push(path);
    }
    match paths.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err("QEMU command has no memory-backend-file RAM object".into()),
        _ => Err("QEMU command has multiple RAM backing files".into()),
    }
}

fn qemu_ram_file(pid: u32) -> Result<PathBuf, String> {
    let command_line = fs::read(format!("/proc/{pid}/cmdline"))
        .map_err(|error| format!("read QEMU command line for {pid}: {error}"))?;
    let args = command_line
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .collect::<Vec<_>>();
    qemu_ram_file_from_args(&args)
}

pub fn memory_map_clone(id: &str, state_dir: &Path) -> Result<Value, String> {
    let registry = crate::lifecycle::CloneRegistry::open_read_only(state_dir)?;
    let record = registry.get(id)?;
    if record.state != crate::lifecycle::CloneState::Running {
        return Err(format!("clone {id} is {:?}, not running", record.state));
    }
    let pid = record
        .qemu_pid
        .ok_or_else(|| format!("clone {id} has no recorded QEMU PID"))?;
    let expected_ticks = record
        .qemu_start_ticks
        .ok_or_else(|| format!("clone {id} has no QEMU process fingerprint"))?;
    let expected_boot_id = record
        .qemu_boot_id
        .as_deref()
        .ok_or_else(|| format!("clone {id} has no QEMU boot ID"))?;
    if crate::lifecycle::current_boot_id()? != expected_boot_id
        || crate::lifecycle::process_start_ticks(pid)? != Some(expected_ticks)
    {
        return Err(format!(
            "clone {id} QEMU process identity no longer matches its record"
        ));
    }
    let ram_file = qemu_ram_file(pid)?;
    let mut report = memory_map(pid, &ram_file)?;
    if crate::lifecycle::process_start_ticks(pid)? != Some(expected_ticks) {
        return Err(format!(
            "clone {id} QEMU process changed while sampling RAM"
        ));
    }
    report["clone"] = json!({
        "id":record.id,
        "generation_id":record.generation_id,
        "qemu_pid":pid,
        "ram_file":ram_file,
    });
    Ok(report)
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
    use super::{
        plan_template_gc, plan_template_gc_report, template_generation, template_tree,
        validate_template_generation, validate_template_tree,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn verified_tree(nodes: &[(&str, Option<&str>)]) -> serde_json::Value {
        serde_json::json!({
            "status": "verified",
            "integrity_verified": true,
            "generations": nodes.iter().map(|(id, parent)| serde_json::json!({
                "generation_id": id,
                "parent_generation_id": parent,
                "integrity_verified": true,
                "directory": format!("/catalog/{id}"),
                "manifest_path": format!("/catalog/{id}/generation.json"),
            })).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn template_gc_plan_refuses_to_race_an_active_publication() {
        let root = std::env::temp_dir().join(format!(
            "densevm-template-gc-lock-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state_dir = root.join("state");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&state_dir).unwrap();
        let _publication_lock =
            crate::template_lock::TemplateTreeLock::acquire_exclusive(&root, false).unwrap();

        let error = plan_template_gc(&root, &state_dir, &[]).unwrap_err();
        assert!(error.contains("template-tree operation is already active"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn template_gc_recovery_finishes_a_journaled_partial_unlink() {
        let root = std::env::temp_dir().join(format!(
            "densevm-template-gc-recover-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let catalog = root.join("catalog");
        let child = catalog.join("leaf");
        let state_dir = root.join("state");
        fs::create_dir_all(&child).unwrap();
        fs::create_dir_all(&state_dir).unwrap();
        let generation_id = format!("sha256:{}", "a".repeat(64));
        let artifacts = [
            ("disk", "disk.qcow2", "disk payload"),
            ("machine_state", "machine-state", "state payload"),
            ("ram", "memory.img", "RAM payload"),
        ];
        let mut journal_artifacts = Vec::new();
        for (kind, name, contents) in artifacts {
            fs::write(child.join(name), contents).unwrap();
            journal_artifacts.push(serde_json::json!({
                "kind":kind,
                "name":name,
                "sha256":digest(&child.join(name)),
            }));
        }
        let manifest = serde_json::json!({
            "generation_id":generation_id,
            "disk":{"path":"disk.qcow2"},
            "machine_state":{"path":"machine-state"},
            "ram":{"path":"memory.img"},
        });
        let manifest_path = child.join("generation.json");
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        fs::write(child.join("experiment.log"), "preserve me").unwrap();
        let journal_path = catalog.join(format!(".densevm-gc-{}.json", &generation_id[7..]));
        fs::write(
            &journal_path,
            serde_json::to_vec(&serde_json::json!({
                "schema_version":1,
                "generation_id":generation_id,
                "generation_dir":child,
                "manifest_sha256":digest(&manifest_path),
                "artifacts":journal_artifacts,
            }))
            .unwrap(),
        )
        .unwrap();
        fs::remove_file(child.join("disk.qcow2")).unwrap();
        fs::write(child.join("machine-state"), "unexpected replacement").unwrap();

        let error = super::recover_template_gc(&catalog, &state_dir).unwrap_err();
        assert!(error.contains("GC artifact is unsafe or changed"));
        assert!(journal_path.exists());
        assert!(child.join("memory.img").exists());
        fs::write(child.join("machine-state"), "state payload").unwrap();

        let report = super::recover_template_gc(&catalog, &state_dir).unwrap();

        assert_eq!(report["status"], "recovered");
        assert_eq!(report["recovered"][0]["generation_id"], generation_id);
        assert!(!journal_path.exists());
        assert!(!manifest_path.exists());
        for (_, name, _) in artifacts {
            assert!(!child.join(name).exists());
        }
        assert_eq!(
            fs::read_to_string(child.join("experiment.log")).unwrap(),
            "preserve me"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn template_gc_plan_blocks_while_a_recovery_journal_is_pending() {
        let root = std::env::temp_dir().join(format!(
            "densevm-template-gc-pending-plan-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let catalog = root.join("catalog");
        let state_dir = root.join("state");
        fs::create_dir_all(&catalog).unwrap();
        fs::create_dir_all(&state_dir).unwrap();
        let journal = catalog.join(format!(".densevm-gc-{}.json", "c".repeat(64)));
        fs::write(&journal, "pending recovery").unwrap();

        let report = plan_template_gc(&catalog, &state_dir, &[]).unwrap();

        assert_eq!(report["status"], "blocked");
        assert_eq!(report["candidate_generations"], serde_json::json!([]));
        assert!(
            report["blockers"][0]
                .as_str()
                .unwrap()
                .contains("gc-recover")
        );
        assert!(journal.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn template_gc_recovery_clears_journal_after_manifest_unlink() {
        let root = std::env::temp_dir().join(format!(
            "densevm-template-gc-recover-finalize-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let catalog = root.join("catalog");
        let child = catalog.join("leaf");
        let state_dir = root.join("state");
        fs::create_dir_all(&child).unwrap();
        fs::create_dir_all(&state_dir).unwrap();
        let generation_id = format!("sha256:{}", "b".repeat(64));
        let artifacts = [
            ("disk", "disk.qcow2", "disk"),
            ("machine_state", "machine-state", "state"),
            ("ram", "memory.img", "ram"),
        ];
        let mut entries = Vec::new();
        for (kind, name, contents) in artifacts {
            fs::write(child.join(name), contents).unwrap();
            entries.push(
                serde_json::json!({"kind":kind,"name":name,"sha256":digest(&child.join(name))}),
            );
        }
        let manifest = serde_json::json!({
            "generation_id":generation_id,
            "disk":{"path":"disk.qcow2"},
            "machine_state":{"path":"machine-state"},
            "ram":{"path":"memory.img"},
        });
        let manifest_path = child.join("generation.json");
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let journal_path = catalog.join(format!(".densevm-gc-{}.json", &generation_id[7..]));
        fs::write(
            &journal_path,
            serde_json::to_vec(&serde_json::json!({
                "schema_version":1,
                "generation_id":generation_id,
                "generation_dir":child,
                "manifest_sha256":digest(&manifest_path),
                "artifacts":entries,
            }))
            .unwrap(),
        )
        .unwrap();
        for (_, name, _) in artifacts {
            fs::remove_file(child.join(name)).unwrap();
        }
        fs::remove_file(manifest_path).unwrap();

        let report = super::recover_template_gc(&catalog, &state_dir).unwrap();

        assert_eq!(report["status"], "recovered");
        assert!(!journal_path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn template_gc_deletes_only_verified_leaf_artifacts_and_keeps_other_files() {
        let root = std::env::temp_dir().join(format!(
            "densevm-template-gc-delete-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let catalog = root.join("catalog");
        let parent = root.join("parent");
        let child = catalog.join("leaf");
        let state_dir = root.join("state");
        fs::create_dir_all(&child).unwrap();
        fs::create_dir_all(&parent).unwrap();
        fs::create_dir_all(&state_dir).unwrap();
        for (name, contents) in [
            ("result.json", "parent result"),
            ("memory.img", "parent RAM"),
            ("machine-state", "parent state"),
            ("base.qcow2", "base disk"),
        ] {
            fs::write(parent.join(name), contents).unwrap();
        }
        for (name, contents) in [
            ("child-memory.img", "child RAM"),
            ("child-machine-state", "child state"),
            ("parent.qcow2", "child disk"),
        ] {
            fs::write(child.join(name), contents).unwrap();
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
        let manifest = serde_json::json!({
            "schema_version": 3,
            "generation_id": generation_id,
            "parent": {
                "template_dir": parent,
                "artifacts": {
                    "result": {"path":"result.json", "sha256":parent_hashes[0]},
                    "ram": {"path":"memory.img", "sha256":parent_hashes[1]},
                    "machine_state": {"path":"machine-state", "sha256":parent_hashes[2]},
                    "disk": {"path":"base.qcow2", "sha256":parent_hashes[3]},
                },
            },
            "ram": {"path":"child-memory.img", "sha256":child_hashes[0], "logical_size_bytes":12},
            "machine_state": {"path":"child-machine-state", "sha256":child_hashes[1]},
            "disk": {
                "path":"parent.qcow2", "sha256":child_hashes[2],
                "backing_path":parent.join("base.qcow2"),
                "backing_sha256":parent_hashes[3],
            },
        });
        fs::write(
            child.join("generation.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(child.join("qemu.log"), "keep experiment output").unwrap();
        let parent_disk_hash = digest(&parent.join("base.qcow2"));
        let keep_ids = [generation_id.clone()];
        let protected =
            super::delete_template_gc_leaf(&catalog, &state_dir, &generation_id, &keep_ids)
                .unwrap();
        assert_eq!(protected["status"], "blocked");
        assert!(child.join("generation.json").exists());
        let lock_probe = crate::template_lock::TemplateTreeLock::acquire_exclusive(&catalog, true)
            .expect("first GC call must release its catalog lock");
        drop(lock_probe);

        let report =
            super::delete_template_gc_leaf(&catalog, &state_dir, &generation_id, &[]).unwrap();

        assert_eq!(report["status"], "deleted");
        assert_eq!(report["generation_id"], generation_id);
        assert_eq!(report["deleted_files"].as_array().unwrap().len(), 4);
        for name in [
            "generation.json",
            "child-memory.img",
            "child-machine-state",
            "parent.qcow2",
        ] {
            assert!(!child.join(name).exists());
        }
        assert_eq!(
            fs::read_to_string(child.join("qemu.log")).unwrap(),
            "keep experiment output"
        );
        assert_eq!(digest(&parent.join("base.qcow2")), parent_disk_hash);
        assert_eq!(
            report["unrecognized_entries_preserved"],
            serde_json::json!([".densevm-template-tree.lock", "qemu.log"])
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn template_gc_protects_kept_generation_and_its_ancestors() {
        let report = plan_template_gc_report(
            verified_tree(&[
                ("root", None),
                ("web", Some("root")),
                ("web-child", Some("web")),
                ("db", None),
            ]),
            &["web-child".to_string()],
            &[],
        );

        assert_eq!(report["status"], "planned");
        assert_eq!(
            report["protected_generation_ids"],
            serde_json::json!(["root", "web", "web-child"])
        );
        assert_eq!(
            report["candidate_generation_ids"],
            serde_json::json!(["db"])
        );
        assert_eq!(report["candidate_leaves"], serde_json::json!(["db"]));
        assert_eq!(report["candidate_generations"][0]["generation_id"], "db");
        assert_eq!(
            report["candidate_generations"][0]["directory"],
            "/catalog/db"
        );
        assert_eq!(
            report["candidate_generations"][0]["manifest_path"],
            "/catalog/db/generation.json"
        );
        assert_eq!(report["candidate_generations"][0]["is_leaf"], true);
        assert_eq!(report["dry_run"], true);
    }

    #[test]
    fn template_gc_protects_active_clone_generation_and_its_ancestors() {
        let report = plan_template_gc_report(
            verified_tree(&[("root", None), ("running", Some("root")), ("orphan", None)]),
            &[],
            &["running".to_string()],
        );

        assert_eq!(report["status"], "planned");
        assert_eq!(
            report["protected_generation_ids"],
            serde_json::json!(["root", "running"])
        );
        assert_eq!(
            report["candidate_generation_ids"],
            serde_json::json!(["orphan"])
        );
    }

    #[test]
    fn template_gc_blocks_candidates_when_tree_integrity_is_unverified() {
        let mut tree = verified_tree(&[("root", None), ("leaf", Some("root"))]);
        tree["status"] = serde_json::json!("failed");
        tree["integrity_verified"] = serde_json::json!(false);

        let report = plan_template_gc_report(tree, &[], &[]);

        assert_eq!(report["status"], "blocked");
        assert_eq!(report["candidate_generation_ids"], serde_json::json!([]));
    }

    #[test]
    fn template_gc_blocks_candidates_for_unknown_clone_generation() {
        let report = plan_template_gc_report(
            verified_tree(&[("root", None), ("leaf", Some("root"))]),
            &[],
            &["missing".to_string()],
        );

        assert_eq!(report["status"], "blocked");
        assert_eq!(report["candidate_generation_ids"], serde_json::json!([]));
        assert!(report["blockers"][0].as_str().unwrap().contains("missing"));
    }

    #[test]
    fn template_gc_blocks_cycles_and_duplicate_generation_ids() {
        for tree in [
            verified_tree(&[("a", Some("b")), ("b", Some("a"))]),
            verified_tree(&[("a", None), ("a", None)]),
        ] {
            let report = plan_template_gc_report(tree, &[], &[]);
            assert_eq!(report["status"], "blocked");
            assert_eq!(report["candidate_generation_ids"], serde_json::json!([]));
        }
    }

    #[test]
    fn template_gc_blocks_invalid_parent_references_and_missing_catalog_paths() {
        let mut invalid_parent = verified_tree(&[("root", None)]);
        invalid_parent["generations"][0]["parent_generation_id"] = serde_json::json!(17);
        let mut missing_path = verified_tree(&[("root", None)]);
        missing_path["generations"][0]
            .as_object_mut()
            .unwrap()
            .remove("directory");

        for tree in [invalid_parent, missing_path] {
            let report = plan_template_gc_report(tree, &[], &[]);
            assert_eq!(report["status"], "blocked");
            assert_eq!(report["candidate_generations"], serde_json::json!([]));
            assert_eq!(report["candidate_generation_ids"], serde_json::json!([]));
        }
    }

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
        fs::remove_file(root.join("generation.json")).unwrap();
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
        let tree_report = validate_template_tree(&root).unwrap();
        assert_eq!(tree_report["status"], "verified");
        assert_eq!(tree_report["integrity_verified"], true);
        assert_eq!(
            tree_report["generations"][0]["validation"]["status"],
            "verified"
        );

        let state_dir = root.join("clone-registry");
        let registry = crate::lifecycle::CloneRegistry::open(&state_dir).unwrap();
        let make_clone = |id: &str,
                          slot: u8,
                          generation_id: String,
                          state: crate::lifecycle::CloneState,
                          qemu_pid: Option<u32>| {
            crate::lifecycle::CloneRecord {
                schema_version: 1,
                id: id.into(),
                generation_id,
                qemu_pid,
                qemu_start_ticks: None,
                qemu_boot_id: None,
                launcher_pid: None,
                launcher_start_ticks: None,
                launcher_boot_id: None,
                qmp_socket: root.join(format!("{id}.qmp")),
                disk_overlay: root.join(format!("{id}.qcow2")),
                machine_id: format!("{slot:032x}"),
                mac: format!("02:00:00:00:00:{slot:02x}"),
                state,
            }
        };
        for (slot, (id, state, pid, template_id)) in [
            (
                "active-running",
                crate::lifecycle::CloneState::Running,
                Some(12345),
                generation_id.clone(),
            ),
            (
                "active-starting",
                crate::lifecycle::CloneState::Starting,
                None,
                generation_id.clone(),
            ),
            (
                "inactive-stopped",
                crate::lifecycle::CloneState::Stopped,
                None,
                format!("sha256:{}", "f".repeat(64)),
            ),
            (
                "inactive-failed",
                crate::lifecycle::CloneState::Failed,
                None,
                format!("sha256:{}", "e".repeat(64)),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            registry
                .create(&make_clone(id, slot as u8 + 1, template_id, state, pid))
                .unwrap();
        }

        let gc_plan = super::plan_template_gc(&root, &state_dir, &[]).unwrap();
        assert_eq!(gc_plan["status"], "planned");
        let active_ids = gc_plan["active_clones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|clone| clone["clone_id"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(active_ids, ["active-running", "active-starting"]);
        assert_eq!(gc_plan["active_clones"][0]["state"], "running");
        assert_eq!(gc_plan["active_clones"][1]["state"], "starting");
        assert_eq!(gc_plan["protected_generation_ids"][0], generation_id);
        assert_eq!(gc_plan["candidate_generation_ids"], serde_json::json!([]));

        fs::write(child.join("child-memory.img"), "modified child ram").unwrap();
        assert!(
            validate_template_generation(&manifest_path)
                .unwrap_err()
                .contains("hash mismatch")
        );
        let tree_report = validate_template_tree(&root).unwrap();
        assert_eq!(tree_report["status"], "failed");
        assert_eq!(tree_report["integrity_verified"], false);
        assert_eq!(
            tree_report["generations"][0]["validation"]["status"],
            "failed"
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

    #[test]
    fn marks_empty_template_tree_unverified() {
        let (root, manifest) = fixture("{}");
        fs::remove_file(manifest).unwrap();
        let report = validate_template_tree(&root).unwrap();
        assert_eq!(report["status"], "empty");
        assert_eq!(report["integrity_verified"], false);
        assert_eq!(report["validated_generation_count"], 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reports_duplicate_generation_ids_in_template_tree() {
        let (root, manifest) = fixture("{}");
        fs::remove_file(manifest).unwrap();
        let metadata = serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "generation_id": "sha256:duplicate",
            "parent": {"template_dir": "/external/root"},
            "ram": {"path": "ram.img", "sha256": "ram-hash", "logical_size_bytes": 12},
            "machine_state": {"path": "state", "sha256": "state-hash"},
            "disk": {"path": "disk.qcow2", "sha256": "disk-hash", "backing_path": "/external/root/base.qcow2"}
        }))
        .unwrap();
        for name in ["generation-a", "generation-b"] {
            let directory = root.join(name);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("generation.json"), &metadata).unwrap();
        }

        let report = validate_template_tree(&root).unwrap();
        assert_eq!(report["status"], "failed");
        assert_eq!(report["integrity_verified"], false);
        assert_eq!(report["duplicate_generation_ids"][0], "sha256:duplicate");
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod memory_map_tests {
    use super::qemu_ram_file_from_args;
    use std::path::Path;

    #[test]
    fn resolves_persistent_clone_disk_only_inside_its_instance_directory() {
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};

        let root = std::env::temp_dir().join(format!(
            "densevm-clone-disk-inspect-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let instance = root.join("alpha");
        fs::create_dir_all(&instance).unwrap();
        let overlay = instance.join("disk.qcow2");
        fs::write(&overlay, b"overlay").unwrap();

        assert_eq!(
            super::clone_disk_path("alpha", &root, &overlay).unwrap(),
            overlay.canonicalize().unwrap()
        );
        assert!(super::clone_disk_path("alpha", &root, &root.join("outside.qcow2")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolves_one_absolute_ram_file_from_qemu_object_arguments() {
        let args: [&[u8]; 5] = [
            b"qemu-system-x86_64",
            b"-object",
            b"memory-backend-file,id=ram,mem-path=/tmp/template/memory.img,size=256M,share=off",
            b"-drive",
            b"if=none,id=disk,file=/tmp/clone.qcow2",
        ];
        assert_eq!(
            qemu_ram_file_from_args(&args).unwrap(),
            Path::new("/tmp/template/memory.img")
        );
    }

    #[test]
    fn rejects_ambiguous_or_missing_qemu_ram_objects() {
        let one = b"memory-backend-file,id=ram,mem-path=/tmp/template/memory.img,size=256M";
        let duplicate_args: [&[u8]; 5] = [b"qemu", b"-object", one, b"-object", one];
        assert!(qemu_ram_file_from_args(&duplicate_args).is_err());
        let missing_path: [&[u8]; 3] =
            [b"qemu", b"-object", b"memory-backend-file,id=ram,size=256M"];
        assert!(qemu_ram_file_from_args(&missing_path).is_err());
        let relative_path: [&[u8]; 3] = [
            b"qemu",
            b"-object",
            b"memory-backend-file,id=ram,mem-path=memory.img,size=256M",
        ];
        assert!(qemu_ram_file_from_args(&relative_path).is_err());
    }
}
