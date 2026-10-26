use serde_json::Value;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug)]
pub struct TemplateArtifacts {
    pub generation_id: String,
    pub ram: PathBuf,
    pub machine_state: PathBuf,
    pub disk: PathBuf,
    pub logical_ram_bytes: u64,
    pub qemu_command: Vec<String>,
}

pub fn load_template_artifacts(manifest_path: &Path) -> Result<TemplateArtifacts, String> {
    let report = crate::inspect::validate_template_generation(manifest_path)?;
    if report["schema_version"] != 4 {
        return Err("clone runtime requires a relocatable v4 template generation".into());
    }
    let manifest_path = manifest_path
        .canonicalize()
        .map_err(|error| format!("resolve template manifest: {error}"))?;
    let contents = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("read template manifest: {error}"))?;
    let manifest: Value = serde_json::from_str(&contents)
        .map_err(|error| format!("parse template manifest: {error}"))?;
    parse_template_manifest(&manifest_path, &manifest, &report)
}

pub fn create_disk_overlay(template_disk: &Path, instance_dir: &Path) -> Result<PathBuf, String> {
    if !instance_dir.is_dir() {
        return Err(format!(
            "clone instance directory does not exist: {}",
            instance_dir.display()
        ));
    }
    let template_disk = template_disk
        .canonicalize()
        .map_err(|error| format!("resolve template disk {}: {error}", template_disk.display()))?;
    if !template_disk.is_file() {
        return Err("template disk is not a regular file".into());
    }
    let overlay = instance_dir.join("disk.qcow2");
    if overlay.exists() {
        return Err(format!(
            "clone overlay already exists: {}",
            overlay.display()
        ));
    }
    let created = Command::new("qemu-img")
        .args(["create", "-f", "qcow2", "-F", "qcow2", "-b"])
        .arg(&template_disk)
        .arg(&overlay)
        .output()
        .map_err(|error| format!("start qemu-img to create clone overlay: {error}"))?;
    if !created.status.success() {
        return Err(format!(
            "qemu-img failed to create clone overlay: {}",
            String::from_utf8_lossy(&created.stderr)
        ));
    }
    let result = verify_disk_overlay(&template_disk, &overlay);
    if result.is_err() {
        let _ = fs::remove_file(&overlay);
    }
    result.map(|()| overlay)
}

fn verify_disk_overlay(template_disk: &Path, overlay: &Path) -> Result<(), String> {
    let info = Command::new("qemu-img")
        .args(["info", "--output=json"])
        .arg(overlay)
        .output()
        .map_err(|error| format!("inspect clone overlay: {error}"))?;
    if !info.status.success() {
        return Err(format!(
            "qemu-img info failed for clone overlay: {}",
            String::from_utf8_lossy(&info.stderr)
        ));
    }
    let metadata: Value = serde_json::from_slice(&info.stdout)
        .map_err(|error| format!("parse clone overlay metadata: {error}"))?;
    let backing = metadata["full-backing-filename"]
        .as_str()
        .or_else(|| metadata["backing-filename"].as_str())
        .ok_or("clone overlay has no backing filename")?;
    if Path::new(backing).canonicalize().ok().as_deref() != Some(template_disk) {
        return Err(format!(
            "clone overlay backing path mismatch: expected {}, found {backing}",
            template_disk.display()
        ));
    }
    if metadata["format"] != "qcow2" || metadata["virtual-size"].as_u64().unwrap_or(0) == 0 {
        return Err("clone overlay is not a nonempty qcow2 image".into());
    }
    let checked = Command::new("qemu-img")
        .arg("check")
        .arg(overlay)
        .output()
        .map_err(|error| format!("check clone overlay: {error}"))?;
    if !checked.status.success() {
        return Err(format!(
            "qemu-img check failed for clone overlay: {}",
            String::from_utf8_lossy(&checked.stderr)
        ));
    }
    Ok(())
}

fn parse_template_manifest(
    manifest_path: &Path,
    manifest: &Value,
    report: &Value,
) -> Result<TemplateArtifacts, String> {
    fn artifact_path(
        manifest_path: &Path,
        manifest: &Value,
        section: &str,
    ) -> Result<PathBuf, String> {
        let filename = manifest[section]["path"]
            .as_str()
            .ok_or_else(|| format!("template manifest is missing {section}.path"))?;
        let relative = Path::new(filename);
        if relative.components().count() != 1
            || !matches!(relative.components().next(), Some(Component::Normal(_)))
        {
            return Err(format!("template {section} path must be a local filename"));
        }
        Ok(manifest_path
            .parent()
            .ok_or("template manifest has no parent directory")?
            .join(relative))
    }

    if manifest["schema_version"] != 4 {
        return Err("clone runtime requires a relocatable v4 template generation".into());
    }
    let qemu_command = manifest["qemu_command"]
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or("template manifest has no QEMU command")?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| "template QEMU command contains a non-string argument".into())
        })
        .collect::<Result<Vec<_>, String>>()?;
    let logical_ram_bytes = manifest["ram"]["logical_size_bytes"]
        .as_u64()
        .filter(|size| *size > 0)
        .ok_or("template manifest has invalid logical RAM size")?;
    let generation_id = report["generation_id"]
        .as_str()
        .ok_or("verified template report omitted generation ID")?
        .to_string();
    Ok(TemplateArtifacts {
        generation_id,
        ram: artifact_path(manifest_path, manifest, "ram")?,
        machine_state: artifact_path(manifest_path, manifest, "machine_state")?,
        disk: artifact_path(manifest_path, manifest, "disk")?,
        logical_ram_bytes,
        qemu_command,
    })
}

pub fn build_clone_command(
    source: &[String],
    clone_id: &str,
    ram_template: &Path,
    disk_overlay: &Path,
    instance_dir: &Path,
) -> Result<Vec<String>, String> {
    validate_component(clone_id)?;
    if source.is_empty() || source[0].is_empty() {
        return Err("template QEMU command is empty".into());
    }
    let ram_path = option_path(ram_template, "template RAM")?;
    let disk_path = option_path(disk_overlay, "clone overlay")?;
    let qmp_path = instance_dir.join(format!("{clone_id}.qmp.sock"));
    let qmp_path = option_path(&qmp_path, "QMP socket")?;
    if qmp_path.len() >= 108 {
        return Err("QMP socket path exceeds the Linux Unix-socket limit".into());
    }

    let mut command = Vec::with_capacity(source.len() + 4);
    let mut ram_backends = 0;
    let mut disks = 0;
    let mut qmp_sockets = 0;
    let mut serial_sockets = 0;
    let mut has_name = false;
    let mut index = 0;
    while index < source.len() {
        let value = &source[index];
        match value.as_str() {
            "-incoming" | "-netdev" | "-nic" | "-net" => {
                require_pair(source, index)?;
                index += 2;
            }
            "-device" => {
                let device = source
                    .get(index + 1)
                    .ok_or("template QEMU command has -device without a value")?;
                if is_network_device(device) {
                    index += 2;
                } else {
                    command.extend([value.clone(), device.clone()]);
                    index += 2;
                }
            }
            "-name" => {
                require_pair(source, index)?;
                if has_name {
                    return Err("template QEMU command has multiple -name options".into());
                }
                command.extend(["-name".into(), clone_id.into()]);
                has_name = true;
                index += 2;
            }
            "-object" => {
                let object = source
                    .get(index + 1)
                    .ok_or("template QEMU command has -object without a value")?;
                if object.starts_with("memory-backend-file,id=ram,") {
                    ram_backends += 1;
                    if ram_backends > 1 {
                        return Err("template QEMU command has multiple RAM backends".into());
                    }
                    command.extend([value.clone(), private_ram_backend(object, ram_path)?]);
                } else {
                    command.extend([value.clone(), object.clone()]);
                }
                index += 2;
            }
            "-drive" => {
                let drive = source
                    .get(index + 1)
                    .ok_or("template QEMU command has -drive without a value")?;
                if drive.split(',').any(|part| part == "id=disk") {
                    disks += 1;
                    if disks > 1 {
                        return Err(
                            "template QEMU command has multiple disk drives with id=disk".into(),
                        );
                    }
                    command.extend([value.clone(), private_disk(drive, disk_path)?]);
                } else {
                    command.extend([value.clone(), drive.clone()]);
                }
                index += 2;
            }
            "-chardev" => {
                let chardev = source
                    .get(index + 1)
                    .ok_or("template QEMU command has -chardev without a value")?;
                if let Some((rewritten, id)) = private_chardev(chardev, instance_dir, clone_id)? {
                    if id == "serial" {
                        serial_sockets += 1;
                    }
                    command.extend([value.clone(), rewritten]);
                } else {
                    command.extend([value.clone(), chardev.clone()]);
                }
                index += 2;
            }
            "-qmp" => {
                require_pair(source, index)?;
                qmp_sockets += 1;
                if qmp_sockets > 1 {
                    return Err("template QEMU command has multiple QMP endpoints".into());
                }
                command.extend([value.clone(), format!("unix:{qmp_path},server=on,wait=off")]);
                index += 2;
            }
            _ => {
                command.push(value.clone());
                index += 1;
            }
        }
    }
    if ram_backends != 1 || disks != 1 || qmp_sockets != 1 || serial_sockets != 1 {
        return Err(format!(
            "template command requires one RAM backend, disk, QMP endpoint, and serial socket (found {ram_backends}, {disks}, {qmp_sockets}, {serial_sockets})"
        ));
    }
    if !has_name {
        command.extend(["-name".into(), clone_id.into()]);
    }
    command.extend(["-incoming".into(), "defer".into()]);
    Ok(command)
}

fn validate_component(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("clone ID must be 1-64 ASCII letters, digits, '-' or '_'".into());
    }
    Ok(())
}

fn option_path<'a>(path: &'a Path, label: &str) -> Result<&'a str, String> {
    if !path.is_absolute() {
        return Err(format!("{label} path must be absolute"));
    }
    let value = path
        .to_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{label} path must be nonempty UTF-8"))?;
    if value.contains(',') || value.contains('=') {
        return Err(format!(
            "{label} path contains unsupported QEMU option characters"
        ));
    }
    Ok(value)
}

fn require_pair(source: &[String], index: usize) -> Result<(), String> {
    if source.get(index + 1).is_none() {
        Err(format!(
            "template QEMU command has {} without a value",
            source[index]
        ))
    } else {
        Ok(())
    }
}

fn private_ram_backend(source: &str, ram_path: &str) -> Result<String, String> {
    let fields = source.split(',').collect::<Vec<_>>();
    let size = fields
        .iter()
        .find(|field| field.starts_with("size=") && field.len() > 5)
        .ok_or("template RAM backend has no size")?;
    let merge = fields
        .iter()
        .find(|field| field.starts_with("merge="))
        .copied();
    let mut result = format!(
        "memory-backend-file,id=ram,mem-path={ram_path},{size},share=off,readonly=on,rom=off"
    );
    if let Some(merge) = merge {
        result.push(',');
        result.push_str(merge);
    }
    Ok(result)
}

fn private_disk(source: &str, disk_path: &str) -> Result<String, String> {
    let mut fields = source.split(',').collect::<Vec<_>>();
    if !fields.iter().any(|field| *field == "format=qcow2") {
        return Err("template disk must use qcow2 format".into());
    }
    let file = fields
        .iter_mut()
        .find(|field| field.starts_with("file="))
        .ok_or("template disk drive has no file path")?;
    *file = "";
    let mut result = Vec::new();
    for field in fields {
        if !field.is_empty() {
            result.push(field.to_string());
        }
    }
    if !result.iter().any(|field| field == "id=disk") {
        return Err("template disk drive has no id=disk".into());
    }
    let file_index = result
        .iter()
        .position(|field| field.starts_with("file="))
        .unwrap_or(result.len());
    result.insert(file_index, format!("file={disk_path}"));
    Ok(result.join(","))
}

fn private_chardev(
    source: &str,
    instance_dir: &Path,
    clone_id: &str,
) -> Result<Option<(String, String)>, String> {
    if !source.starts_with("socket,") {
        return Ok(None);
    }
    let fields = source.split(',').collect::<Vec<_>>();
    let id = fields
        .iter()
        .find_map(|field| field.strip_prefix("id="))
        .ok_or("socket chardev has no id")?;
    validate_component(id).map_err(|_| format!("socket chardev has unsafe ID {id:?}"))?;
    if !fields.iter().any(|field| field.starts_with("path=")) {
        return Err(format!("socket chardev {id} has no path"));
    }
    let socket = instance_dir.join(format!("{clone_id}.{id}.sock"));
    let socket = option_path(&socket, "serial socket")?;
    if socket.len() >= 108 {
        return Err("serial socket path exceeds the Linux Unix-socket limit".into());
    }
    let mut rewritten = Vec::new();
    for field in fields {
        if field.starts_with("path=") {
            rewritten.push(format!("path={socket}"));
        } else {
            rewritten.push(field.to_string());
        }
    }
    Ok(Some((rewritten.join(","), id.to_string())))
}

fn is_network_device(value: &str) -> bool {
    let device = value
        .split(',')
        .next()
        .unwrap_or(value)
        .to_ascii_lowercase();
    device.contains("net")
        || matches!(
            device.as_str(),
            "e1000" | "e1000e" | "rtl8139" | "vmxnet3" | "i82559er" | "ne2k_pci"
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn source() -> Vec<String> {
        [
            "qemu-system-x86_64",
            "-name",
            "template",
            "-object",
            "memory-backend-file,id=ram,mem-path=/templates/memory.img,size=256M,share=on,readonly=off,merge=off",
            "-machine",
            "q35,memory-backend=ram",
            "-drive",
            "if=none,id=disk,file=/templates/base.qcow2,format=qcow2",
            "-chardev",
            "socket,id=serial,path=/tmp/template.serial.sock,server=on,wait=off",
            "-serial",
            "chardev:serial",
            "-qmp",
            "unix:/tmp/template.qmp.sock,server=on,wait=off",
            "-netdev",
            "user,id=net0",
            "-device",
            "virtio-net-pci,netdev=net0,mac=02:00:00:00:00:01",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    #[test]
    fn creates_a_small_verified_qcow2_overlay_over_the_template_disk() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("densevm-overlay-{}-{nonce}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let base = root.join("base.qcow2");
        let output = Command::new("qemu-img")
            .args(["create", "-f", "qcow2"])
            .arg(&base)
            .arg("16M")
            .output()
            .unwrap();
        assert!(output.status.success());
        let instance = root.join("clone");
        fs::create_dir(&instance).unwrap();

        let overlay = create_disk_overlay(&base, &instance).unwrap();
        let info = Command::new("qemu-img")
            .args(["info", "--output=json"])
            .arg(&overlay)
            .output()
            .unwrap();
        let info: Value = serde_json::from_slice(&info.stdout).unwrap();
        assert_eq!(info["format"], "qcow2");
        assert_eq!(info["virtual-size"], 16 * 1024 * 1024);
        assert!(info["actual-size"].as_u64().unwrap() < 1024 * 1024);
        assert!(create_disk_overlay(&base, &instance).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn template_metadata_resolves_only_verified_local_artifacts() {
        let manifest_path = Path::new("/templates/child/generation.json");
        let manifest = json!({
            "schema_version": 4,
            "qemu_command": ["qemu-system-x86_64", "-machine", "q35"],
            "ram": {"path": "child-memory.img", "logical_size_bytes": 268435456},
            "machine_state": {"path": "child-machine-state"},
            "disk": {"path": "parent.qcow2"}
        });
        let report = json!({"schema_version": 4, "generation_id": "sha256:verified"});
        let artifacts = parse_template_manifest(manifest_path, &manifest, &report).unwrap();
        assert_eq!(artifacts.generation_id, "sha256:verified");
        assert_eq!(
            artifacts.ram,
            PathBuf::from("/templates/child/child-memory.img")
        );
        assert_eq!(
            artifacts.machine_state,
            PathBuf::from("/templates/child/child-machine-state")
        );
        assert_eq!(
            artifacts.disk,
            PathBuf::from("/templates/child/parent.qcow2")
        );
        assert_eq!(artifacts.logical_ram_bytes, 268435456);
        assert_eq!(artifacts.qemu_command[0], "qemu-system-x86_64");
    }

    #[test]
    fn template_metadata_rejects_legacy_and_traversing_artifacts() {
        let manifest_path = Path::new("/templates/child/generation.json");
        let mut manifest = json!({
            "schema_version": 4,
            "qemu_command": ["qemu-system-x86_64"],
            "ram": {"path": "../memory.img", "logical_size_bytes": 268435456},
            "machine_state": {"path": "state"},
            "disk": {"path": "disk.qcow2"}
        });
        let report = json!({"schema_version": 4, "generation_id": "sha256:verified"});
        assert!(parse_template_manifest(manifest_path, &manifest, &report).is_err());
        manifest["ram"]["path"] = json!("memory.img");
        manifest["schema_version"] = json!(3);
        assert!(parse_template_manifest(manifest_path, &manifest, &report).is_err());
    }

    #[test]
    fn clone_command_uses_private_ram_and_disk_and_detaches_network() {
        let command = build_clone_command(
            &source(),
            "alpha",
            Path::new("/templates/memory.img"),
            Path::new("/instances/alpha/disk.qcow2"),
            Path::new("/instances/alpha"),
        )
        .unwrap();
        let object = command.iter().position(|arg| arg == "-object").unwrap();
        assert!(command[object + 1].contains("mem-path=/templates/memory.img"));
        assert!(command[object + 1].contains("share=off,readonly=on,rom=off"));
        assert!(command[object + 1].ends_with("merge=off"));
        let drive = command.iter().position(|arg| arg == "-drive").unwrap();
        assert!(command[drive + 1].contains("file=/instances/alpha/disk.qcow2"));
        assert!(!command.iter().any(
            |arg| arg == "-netdev" || arg == "virtio-net-pci,netdev=net0,mac=02:00:00:00:00:01"
        ));
        assert!(command.windows(2).any(|pair| pair == ["-name", "alpha"]));
        assert!(
            command
                .windows(2)
                .any(|pair| pair == ["-incoming", "defer"])
        );
    }

    #[test]
    fn clone_command_fails_closed_when_required_template_devices_are_ambiguous() {
        let mut invalid = source();
        invalid.push("-object".into());
        invalid.push("memory-backend-file,id=ram,mem-path=/duplicate,size=256M".into());
        assert!(
            build_clone_command(
                &invalid,
                "alpha",
                Path::new("/templates/memory.img"),
                Path::new("/instances/alpha/disk.qcow2"),
                Path::new("/instances/alpha"),
            )
            .is_err()
        );
    }
}
