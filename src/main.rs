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
            rtc_clock,
        } => densevm::probe::run_options(
            &out,
            densevm::probe::ProbeOptions {
                count,
                vcpus,
                ksm,
                divergence_pages,
                park_idle,
                divergence_all,
                guest_dir: guest_dir.as_deref(),
                range,
                skip_guest_oom,
                vcpu_register_state,
                lapic_tpr_state,
                rtc_alarm,
                rtc_clock,
            },
        ),
        Command::BenchmarkBaseline {
            out,
            count,
            accelerator,
            trace_translations,
        } => densevm::probe::run_baseline(
            &out,
            count,
            accelerator == densevm::cli::BaselineAccelerator::Tcg,
            trace_translations,
        ),
        Command::InspectDisk { image } => densevm::inspect::disk(&image).and_then(print_json),
        Command::InspectCloneDisk { id, state_dir } => {
            densevm::inspect::inspect_clone_disk(&id, &state_dir).and_then(print_json)
        }
        Command::InspectTemplate { manifest } => {
            densevm::inspect::template_generation(&manifest).and_then(print_json)
        }
        Command::ValidateTemplate { manifest } => {
            densevm::inspect::validate_template_generation(&manifest).and_then(print_json)
        }
        Command::ValidateTemplateTree { root } => densevm::inspect::validate_template_tree(&root)
            .and_then(|report| {
                let verified = report["status"] == "verified";
                print_json(report)?;
                if verified {
                    Ok(())
                } else {
                    Err("template tree validation failed; see per-generation results".into())
                }
            }),
        Command::TemplateTree { root } => {
            densevm::inspect::template_tree(&root).and_then(print_json)
        }
        Command::TemplateGcPlan {
            root,
            state_dir,
            keep_ids,
        } => densevm::inspect::plan_template_gc(&root, &state_dir, &keep_ids).and_then(|report| {
            let planned = report["status"] == "planned";
            print_json(report)?;
            if planned {
                Ok(())
            } else {
                Err("template GC planning blocked; see report blockers".into())
            }
        }),
        Command::TemplateGc {
            root,
            state_dir,
            generation_id,
            keep_ids,
        } => {
            densevm::inspect::delete_template_gc_leaf(&root, &state_dir, &generation_id, &keep_ids)
                .and_then(|report| {
                    let deleted = report["status"] == "deleted";
                    print_json(report)?;
                    if deleted {
                        Ok(())
                    } else {
                        Err("template GC deletion blocked; see report blockers".into())
                    }
                })
        }
        Command::MemoryMap { pid, path } => {
            densevm::inspect::memory_map(pid, &path).and_then(print_json)
        }
        Command::MemoryMapClone { id, state_dir } => {
            densevm::inspect::memory_map_clone(&id, &state_dir).and_then(print_json)
        }
        Command::KsmStatus => densevm::inspect::ksm_status().and_then(print_json),
        Command::CloneList { state_dir } => densevm::lifecycle::CloneRegistry::open(&state_dir)
            .and_then(|registry| registry.list())
            .and_then(|clones| serde_json::to_value(clones).map_err(|error| error.to_string()))
            .and_then(print_json),
        Command::CloneReconcile { state_dir } => {
            densevm::lifecycle::CloneRegistry::open(&state_dir)
                .and_then(|registry| registry.reconcile())
                .and_then(|clones| serde_json::to_value(clones).map_err(|error| error.to_string()))
                .and_then(print_json)
        }
        Command::CloneStop { id, state_dir } => densevm::lifecycle::CloneRegistry::open(&state_dir)
            .and_then(|registry| registry.stop(&id, std::time::Duration::from_secs(10)))
            .and_then(|clone| serde_json::to_value(clone).map_err(|error| error.to_string()))
            .and_then(print_json),
        Command::CloneDelete { id, state_dir } => {
            densevm::lifecycle::CloneRegistry::open(&state_dir)
                .and_then(|registry| registry.delete(&id))
                .map(|()| serde_json::json!({"deleted": id}))
                .and_then(print_json)
        }
        Command::CloneStart {
            id,
            manifest,
            state_dir,
        } => densevm::launcher::start_clone(&manifest, &state_dir, &id)
            .and_then(|clone| serde_json::to_value(clone).map_err(|error| error.to_string()))
            .and_then(print_json),
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
