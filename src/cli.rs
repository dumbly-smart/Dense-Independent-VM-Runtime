use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Doctor,
    Stats {
        result: PathBuf,
    },
    Probe {
        out: PathBuf,
        count: u32,
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
    TemplateTree {
        root: PathBuf,
    },
    MemoryMap {
        pid: u32,
        path: PathBuf,
    },
    KsmStatus,
    CloneList {
        state_dir: PathBuf,
    },
    CloneStop {
        id: String,
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

fn parse_out_count<I>(args: &mut I, default_count: u32) -> Result<(PathBuf, u32), String>
where
    I: Iterator<Item = String>,
{
    let mut out = None;
    let mut count = default_count;
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
            flag => return Err(format!("unknown option {flag}")),
        }
    }
    Ok((out.ok_or("--out is required")?, count))
}

fn parse_probe<I>(args: &mut I) -> Result<Command, String>
where
    I: Iterator<Item = String>,
{
    let mut out = None;
    let mut count = 2;
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
            let (out, count) = parse_out_count(&mut args, 1)?;
            Ok(Command::BenchmarkBaseline { out, count })
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
        Some("validate") if args.next().as_deref() == Some("template") => {
            let manifest = PathBuf::from(args.next().ok_or("template manifest path is required")?);
            if args.next().is_some() {
                return Err("usage: densevm validate template MANIFEST".to_string());
            }
            Ok(Command::ValidateTemplate { manifest })
        }
        Some("template") if args.next().as_deref() == Some("tree") => {
            let root = PathBuf::from(args.next().ok_or("template catalog root is required")?);
            if args.next().is_some() {
                return Err("usage: densevm template tree ROOT".to_string());
            }
            Ok(Command::TemplateTree { root })
        }
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
            if action != "list" {
                return Err("usage: densevm clone list --state-dir DIR | densevm clone stop ID --state-dir DIR".into());
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
        _ => Err("usage: densevm probe --out DIR [--count N] | densevm benchmark baseline --out DIR --count N | densevm inspect IMAGE disk | densevm inspect template MANIFEST | densevm validate template MANIFEST | densevm template tree ROOT | densevm clone list --state-dir DIR | densevm clone stop ID --state-dir DIR | densevm memory-map PID RAM_FILE | densevm ksm status".to_string()),
    }
}
