use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Probe { out: PathBuf, count: u32 },
    BenchmarkBaseline { out: PathBuf, count: u32 },
    InspectDisk { image: PathBuf },
    MemoryMap { pid: u32, path: PathBuf },
    KsmStatus,
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

pub fn parse_args<I, S>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut args = args.into_iter().map(Into::into);
    let _program = args.next().ok_or("program name is required")?;
    match args.next().as_deref() {
        Some("probe") => {
            let (out, count) = parse_out_count(&mut args, 2)?;
            Ok(Command::Probe { out, count })
        }
        Some("benchmark") if args.next().as_deref() == Some("baseline") => {
            let (out, count) = parse_out_count(&mut args, 1)?;
            Ok(Command::BenchmarkBaseline { out, count })
        }
        Some("inspect") => {
            let image = PathBuf::from(args.next().ok_or("image path is required")?);
            if args.next().as_deref() != Some("disk") || args.next().is_some() {
                return Err("usage: densevm inspect IMAGE disk".to_string());
            }
            Ok(Command::InspectDisk { image })
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
        _ => Err("usage: densevm probe --out DIR [--count N] | densevm benchmark baseline --out DIR --count N | densevm inspect IMAGE disk | densevm memory-map PID RAM_FILE | densevm ksm status".to_string()),
    }
}
