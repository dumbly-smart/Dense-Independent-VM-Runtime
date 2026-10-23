use densevm::cli::{Command, parse_args};

fn main() {
    if let Err(error) = densevm::interrupt::install() {
        eprintln!("densevm: {error}");
        std::process::exit(1);
    }
    let command = match parse_args(std::env::args()) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("densevm: {error}");
            std::process::exit(2);
        }
    };
    let result = match command {
        Command::Doctor => print_json(densevm::capabilities::host_report()),
        Command::Stats { result } => densevm::report::from_path(&result).and_then(print_json),
        Command::Probe {
            out,
            count,
            ksm,
            divergence_pages,
            park_idle,
            divergence_all,
            guest_dir,
            range,
        } => densevm::probe::run_options(
            &out,
            densevm::probe::ProbeOptions {
                count,
                ksm,
                divergence_pages,
                park_idle,
                divergence_all,
                guest_dir: guest_dir.as_deref(),
                range,
            },
        ),
        Command::BenchmarkBaseline { out, count } => densevm::probe::run_baseline(&out, count),
        Command::InspectDisk { image } => densevm::inspect::disk(&image).and_then(print_json),
        Command::MemoryMap { pid, path } => {
            densevm::inspect::memory_map(pid, &path).and_then(print_json)
        }
        Command::KsmStatus => densevm::inspect::ksm_status().and_then(print_json),
    };
    if let Err(error) = result {
        eprintln!("densevm probe failed: {error}");
        std::process::exit(1);
    }
}

fn print_json(value: serde_json::Value) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
    );
    Ok(())
}
