use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaselineAccelerator {
    Kvm,
    Tcg,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Doctor,
    Stats {
        result: PathBuf,
    },
    Probe {
        out: PathBuf,
        count: u32,
        vcpus: u8,
        ksm: bool,
        divergence_pages: u32,
        park_idle: bool,
        divergence_all: bool,
        guest_dir: Option<PathBuf>,
        range: bool,
        skip_guest_oom: bool,
        vcpu_register_state: bool,
        lapic_tpr_state: bool,
        rtc_alarm: bool,
    },
    BenchmarkBaseline {
        out: PathBuf,
        count: u32,
        accelerator: BaselineAccelerator,
        trace_translations: bool,
    },
    InspectDisk {
        image: PathBuf,
    },
    InspectTemplate {
        manifest: PathBuf,
    },
    ValidateTemplate {
        manifest: PathBuf,
    },
    ValidateTemplateTree {
        root: PathBuf,
    },
    TemplateTree {
        root: PathBuf,
    },
    TemplateGcPlan {
        root: PathBuf,
        state_dir: PathBuf,
        keep_ids: Vec<String>,
    },
    MemoryMap {
        pid: u32,
        path: PathBuf,
    },
    KsmStatus,
    CloneList {
        state_dir: PathBuf,
    },
    CloneReconcile {
        state_dir: PathBuf,
    },
    CloneStop {
        id: String,
        state_dir: PathBuf,
    },
    CloneDelete {
        id: String,
        state_dir: PathBuf,
    },
    CloneStart {
        id: String,
        manifest: PathBuf,
        state_dir: PathBuf,
    },
}

fn value<I>(args: &mut I, flag: &str) -> Result<String, String>
where
    I: Iterator<Item = String>,
{
    args.next()
        .ok_or_else(|| format!("missing value for {flag}"))
}

fn parse_baseline_options<I>(
    args: &mut I,
    default_count: u32,
) -> Result<(PathBuf, u32, BaselineAccelerator, bool), String>
where
    I: Iterator<Item = String>,
{
    let mut out = None;
    let mut count = default_count;
    let mut accelerator = BaselineAccelerator::Kvm;
    let mut trace_translations = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out = Some(PathBuf::from(value(args, "--out")?)),
            "--count" => {
                count = value(args, "--count")?
                    .parse()
                    .map_err(|_| "--count must be a positive integer".to_string())?;
                if count == 0 {
                    return Err("--count must be a positive integer".to_string());
                }
            }
            "--accel" => {
                accelerator = match value(args, "--accel")?.as_str() {
                    "kvm" => BaselineAccelerator::Kvm,
                    "tcg" => BaselineAccelerator::Tcg,
                    _ => return Err("--accel must be kvm or tcg".to_string()),
                };
            }
            "--trace-translations" => trace_translations = true,
            flag => return Err(format!("unknown option {flag}")),
        }
    }
    if trace_translations && accelerator != BaselineAccelerator::Tcg {
        return Err("--trace-translations requires --accel tcg".into());
    }
    Ok((
        out.ok_or("--out is required")?,
        count,
        accelerator,
        trace_translations,
    ))
}

fn parse_probe<I>(args: &mut I) -> Result<Command, String>
where
    I: Iterator<Item = String>,
{
    let mut out = None;
    let mut count = 2;
    let mut vcpus = 1;
    let mut ksm = false;
    let mut park_idle = false;
    let mut divergence_all = false;
    let mut guest_dir = None;
    let mut range = false;
    let mut skip_guest_oom = false;
    let mut vcpu_register_state = false;
    let mut lapic_tpr_state = false;
    let mut rtc_alarm = false;
    let mut divergence_pages = 32 * 256;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out = Some(PathBuf::from(value(args, "--out")?)),
            "--count" => {
                count = value(args, "--count")?
                    .parse()
                    .map_err(|_| "--count must be a positive integer".to_string())?;
                if count == 0 {
                    return Err("--count must be a positive integer".to_string());
                }
            }
            "--vcpus" => {
                vcpus = value(args, "--vcpus")?
                    .parse::<u8>()
                    .map_err(|_| "--vcpus must be between 1 and 4".to_string())?;
                if !(1..=4).contains(&vcpus) {
                    return Err("--vcpus must be between 1 and 4".into());
                }
            }
            "--ksm" => ksm = true,
            "--park-idle" => park_idle = true,
            "--divergence-all" => divergence_all = true,
            "--guest" => guest_dir = Some(PathBuf::from(value(args, "--guest")?)),
            "--range" => range = true,
            "--skip-guest-oom" => skip_guest_oom = true,
            "--vcpu-register-state" => vcpu_register_state = true,
            "--lapic-tpr-state" => {
                lapic_tpr_state = true;
                vcpu_register_state = true;
            }
            "--rtc-alarm" => rtc_alarm = true,
            "--divergence-mib" => {
                let divergence_mib = value(args, "--divergence-mib")?
                    .parse::<u32>()
                    .map_err(|_| "--divergence-mib must be an integer from 0 to 32".to_string())?;
                if divergence_mib > 32 {
                    return Err("--divergence-mib must be an integer from 0 to 32".to_string());
                }
                divergence_pages = divergence_mib * 256;
            }
            "--divergence-percent" => {
                let percent = value(args, "--divergence-percent")?
                    .parse::<u32>()
                    .map_err(|_| {
                        "--divergence-percent must be an integer from 0 to 100".to_string()
                    })?;
                if percent > 100 {
                    return Err("--divergence-percent must be an integer from 0 to 100".to_string());
                }
                divergence_pages = (32 * 1024 / 4 * percent + 50) / 100;
            }
            flag => return Err(format!("unknown option {flag}")),
        }
    }
    Ok(Command::Probe {
        out: out.ok_or("--out is required")?,
        count,
        vcpus,
        ksm,
        divergence_pages,
        park_idle,
        divergence_all,
        guest_dir,
        range,
        skip_guest_oom,
        vcpu_register_state,
        lapic_tpr_state,
        rtc_alarm,
    })
}

pub fn parse_args<I, S>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut args = args.into_iter().map(Into::into);
    let _program = args.next().ok_or("program name is required")?;
    match args.next().as_deref() {
        Some("doctor") if args.next().is_none() => Ok(Command::Doctor),
        Some("stats") => {
            let result = PathBuf::from(args.next().ok_or("result JSON path is required")?);
            if args.next().is_some() { return Err("usage: densevm stats RESULT_JSON".into()); }
            Ok(Command::Stats { result })
        },
        Some("probe") => {
            parse_probe(&mut args)
        }
        Some("benchmark") if args.next().as_deref() == Some("baseline") => {
            let (out, count, accelerator, trace_translations) =
                parse_baseline_options(&mut args, 1)?;
            Ok(Command::BenchmarkBaseline {
                out,
                count,
                accelerator,
                trace_translations,
            })
        }
        Some("inspect") => {
            let first = args.next().ok_or("image path or inspect target is required")?;
            if first == "template" {
                let manifest = PathBuf::from(args.next().ok_or("template manifest path is required")?);
                if args.next().is_some() {
                    return Err("usage: densevm inspect template MANIFEST".to_string());
                }
                Ok(Command::InspectTemplate { manifest })
            } else {
                let image = PathBuf::from(first);
                if args.next().as_deref() != Some("disk") || args.next().is_some() {
                    return Err("usage: densevm inspect IMAGE disk".to_string());
                }
                Ok(Command::InspectDisk { image })
            }
        }
        Some("validate") => match args.next().as_deref() {
            Some("template") => {
                let manifest =
                    PathBuf::from(args.next().ok_or("template manifest path is required")?);
                if args.next().is_some() {
                    return Err("usage: densevm validate template MANIFEST".to_string());
                }
                Ok(Command::ValidateTemplate { manifest })
            }
            Some("template-tree") => {
                let root =
                    PathBuf::from(args.next().ok_or("template catalog root is required")?);
                if args.next().is_some() {
                    return Err("usage: densevm validate template-tree ROOT".to_string());
                }
                Ok(Command::ValidateTemplateTree { root })
            }
            _ => Err(
                "usage: densevm validate template MANIFEST | densevm validate template-tree ROOT"
                    .to_string(),
            ),
        },
        Some("template") => match args.next().as_deref() {
            Some("tree") => {
                let root = PathBuf::from(args.next().ok_or("template catalog root is required")?);
                if args.next().is_some() {
                    return Err("usage: densevm template tree ROOT".to_string());
                }
                Ok(Command::TemplateTree { root })
            }
            Some("gc-plan") => {
                let root = PathBuf::from(args.next().ok_or("template catalog root is required")?);
                let mut state_dir = None;
                let mut keep_ids = Vec::new();
                while let Some(arg) = args.next() {
                    match arg.as_str() {
                        "--state-dir" => state_dir = Some(PathBuf::from(value(&mut args, "--state-dir")?)),
                        "--keep" => keep_ids.push(value(&mut args, "--keep")?),
                        flag => return Err(format!("unknown option {flag}")),
                    }
                }
                Ok(Command::TemplateGcPlan {
                    root,
                    state_dir: state_dir.ok_or("--state-dir is required")?,
                    keep_ids,
                })
            }
            _ => Err("usage: densevm template tree ROOT | densevm template gc-plan ROOT --state-dir DIR [--keep GENERATION_ID]".into()),
        },
        Some("memory-map") => {
            let pid = args.next().ok_or("PID is required")?.parse().map_err(|_| "PID must be an integer")?;
            let path = PathBuf::from(args.next().ok_or("RAM path is required")?);
            if args.next().is_some() {
                return Err("usage: densevm memory-map PID RAM_FILE".to_string());
            }
            Ok(Command::MemoryMap { pid, path })
        }
        Some("ksm") if args.next().as_deref() == Some("status") && args.next().is_none() => Ok(Command::KsmStatus),
        Some("clone") => {
            let action = args.next().ok_or("clone action is required")?;
            if action == "start" {
                let id = args.next().ok_or("clone ID is required")?;
                let mut manifest = None;
                let mut state_dir = None;
                while let Some(arg) = args.next() {
                    match arg.as_str() {
                        "--template" => manifest = Some(PathBuf::from(value(&mut args, "--template")?)),
                        "--state-dir" => state_dir = Some(PathBuf::from(value(&mut args, "--state-dir")?)),
                        flag => return Err(format!("unknown option {flag}")),
                    }
                }
                return Ok(Command::CloneStart {
                    id,
                    manifest: manifest.ok_or("--template is required")?,
                    state_dir: state_dir.ok_or("--state-dir is required")?,
                });
            }
            if action == "stop" {
                let id = args.next().ok_or("clone ID is required")?;
                let mut state_dir = None;
                while let Some(arg) = args.next() {
                    match arg.as_str() {
                        "--state-dir" => state_dir = Some(PathBuf::from(value(&mut args, "--state-dir")?)),
                        flag => return Err(format!("unknown option {flag}")),
                    }
                }
                return Ok(Command::CloneStop {
                    id,
                    state_dir: state_dir.ok_or("--state-dir is required")?,
                });
            }
            if action == "delete" {
                let id = args.next().ok_or("clone ID is required")?;
                let mut state_dir = None;
                while let Some(arg) = args.next() {
                    match arg.as_str() {
                        "--state-dir" => state_dir = Some(PathBuf::from(value(&mut args, "--state-dir")?)),
                        flag => return Err(format!("unknown option {flag}")),
                    }
                }
                return Ok(Command::CloneDelete {
                    id,
                    state_dir: state_dir.ok_or("--state-dir is required")?,
                });
            }
            if action == "reconcile" {
                let mut state_dir = None;
                while let Some(arg) = args.next() {
                    match arg.as_str() {
                        "--state-dir" => state_dir = Some(PathBuf::from(value(&mut args, "--state-dir")?)),
                        flag => return Err(format!("unknown option {flag}")),
                    }
                }
                return Ok(Command::CloneReconcile {
                    state_dir: state_dir.ok_or("--state-dir is required")?,
                });
            }
            if action != "list" {
                return Err("usage: densevm clone list --state-dir DIR | densevm clone reconcile --state-dir DIR | densevm clone start ID --template MANIFEST --state-dir DIR | densevm clone stop ID --state-dir DIR | densevm clone delete ID --state-dir DIR".into());
            }
            let mut state_dir = None;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--state-dir" => state_dir = Some(PathBuf::from(value(&mut args, "--state-dir")?)),
                    flag => return Err(format!("unknown option {flag}")),
                }
            }
            Ok(Command::CloneList {
                state_dir: state_dir.ok_or("--state-dir is required")?,
            })
        }
        _ => Err("usage: densevm probe --out DIR [--count N] [--vcpus 1..4] | densevm benchmark baseline --out DIR --count N [--accel kvm|tcg] [--trace-translations] | densevm inspect IMAGE disk | densevm inspect template MANIFEST | densevm validate template MANIFEST | densevm template tree ROOT | densevm clone list --state-dir DIR | densevm clone reconcile --state-dir DIR | densevm clone start ID --template MANIFEST --state-dir DIR | densevm clone stop ID --state-dir DIR | densevm clone delete ID --state-dir DIR | densevm memory-map PID RAM_FILE | densevm ksm status".to_string()),
    }
}
