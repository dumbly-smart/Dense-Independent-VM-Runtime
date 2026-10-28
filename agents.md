# Working on DenseVM

This file is the contributor guide for people and coding agents changing this
repository. Read it together with [architecture.md](architecture.md),
[progress.md](progress.md), and the detailed research record
[`docs/progress.md`](docs/progress.md).

## What this project is

DenseVM is a systems research prototype for running separate QEMU/KVM virtual
machines from one frozen Linux machine state while sharing unchanged RAM pages
and immutable disk blocks. Every guest remains a real VM with its own QEMU
process, vCPU state, devices, QMP endpoint, serial console, and writable disk
overlay. The project measures whether sharing helps and where divergence or
host overhead limits the benefit.

It is not a container runtime, guest shell multiplexer, QEMU replacement, or
claim that separate guests' CPU work can be deduplicated.

## Non-negotiable research rules

1. Preserve guest isolation. A guest RAM write must not reach the template RAM
   file or another clone. A disk write must stay in that guest's qcow2 overlay.
2. Use QEMU-supported migration and QMP interfaces. Do not serialize QEMU
   internals by hand.
3. Keep template inputs immutable after freeze. Verify file paths, hashes,
   QEMU/machine compatibility, and disk backing chains where the experiment
   requires them.
4. Use a separate QEMU process per guest. Do not share mutable vCPU or device
   state.
5. Treat guest identity as security-sensitive. Prepare entropy, machine-id,
   DHCP identity, hostname, and host keys before exposing a restored guest to a
   shared network. Record the exact guest profile for which each behavior was
   observed.
6. Measure physical residency from host data. Never infer savings from
   configured guest RAM, virtual disk size, or RSS alone. Report PSS and its
   limits, and include cgroup or kernel counters when available.
7. Keep `UNKNOWN`, `PARTIAL`, `BLOCKED`, and `DEFERRED` distinctions. A passing
   test is evidence for its stated scope, not proof for every Linux
   distribution, QEMU version, machine type, or device.
8. Do not change host-global KSM settings without the required authority and a
   cleanup path that restores the prior value. KSM is optional and can create
   cross-VM side channels.
9. Distinguish KVM from TCG. KVM executes each guest's instructions; it does
   not share guest computation. TCG translation measurements are a separate
   research track.

## Repository map

- `src/`: Rust CLI, QEMU process setup, QMP client, clone lifecycle, identity,
  guest serial protocol, template validation, memory/disk accounting, and
  reports.
- `experiments/`: guest-side Rust agents, benchmark drivers, divergence
  sweeps, KSM comparison, and hierarchical-template experiments.
- `examples/alpine/`: explicit prepared Alpine guest workflow and its service,
  identity, and network profiles.
- `patches/qemu/`: narrowly scoped research patches for QEMU versions whose
  stock trace events do not expose the required TCG metrics.
- `tests/`: deterministic Rust unit/CLI/protocol tests and opt-in
  host-dependent KVM integration tests.
- `results/`: raw JSON, trace logs, and selected artifacts from real runs.
  Raw evidence should be treated as append-only; summarize it elsewhere rather
  than editing it to support a new claim.
- `docs/`: design, semantic-status tables, method notes, and chronological
  experiment history.
- `graft/`: repository code-graph index. For code discovery, use the available
  `graft` graph tools before searching or reading source. Use literal search
  for strings and non-code files.

## Change workflow

1. Read this guide and the relevant design, report, and semantic-status pages.
2. Use graft to locate the implementation and callers before changing Rust
   symbols. Read the exact source span before editing it.
3. State the experiment's hypothesis, measurable outcome, limits, and safe
   resource budget. Do not add an optimization before its mechanism is
   verified.
4. Add a focused regression test for code behavior. Use a real parser or
   protocol where practical; avoid tests that only restate implementation
   details.
5. Run formatting and the narrow test first, then the full Rust suite for
   code changes. For experiments, run the actual QEMU guest and preserve the
   raw output. Clearly mark integration tests that could not run.
6. Check generated JSON against raw trace/log evidence. Never copy or invent
   metrics by hand.
7. Update the relevant documentation and append a dated, concise entry to
   [`docs/progress.md`](docs/progress.md). Keep the root
   [`progress.md`](progress.md) as a readable status summary, not a duplicate
   detailed diary.
8. Make small, focused commits. Push when the task/session authorizes it, and
   verify the resulting branch state.

## Commands

```sh
cargo fmt --all -- --check
cargo test
cargo check
cargo run -- doctor
```

The full host-dependent KVM integration set is opt-in:

```sh
cargo test --test probe -- --ignored --nocapture
```

Run it only when `/dev/kvm`, QEMU, guest artifacts, free memory, and disk
capacity satisfy the test's preconditions. The `doctor` command is read-only.

For ordinary baseline sampling:

```sh
cargo run -- benchmark baseline --out /path/to/run --count 2 --accel kvm
```

For the template/COW probe:

```sh
cargo run -- probe --out /path/to/run --count 2 --skip-guest-oom
```

`cargo run -- template tree ROOT` reads generation and parent-link metadata
without hashing large artifacts. `cargo run -- validate template-tree ROOT`
checks each discovered generation's referenced parent and child hashes and
generation ID; it can read large RAM files and exits nonzero when validation
fails.

The probe defaults to one vCPU. `--vcpus 1..4` applies the same topology to
baseline VMs, the template, and restored clones. Trace output can be large;
`--trace-translations` is restricted to TCG and a small VM count.

## Result review checklist

- Does the JSON identify QEMU, kernel, accelerator, machine, CPU model, guest
  image/profile, and the exact QEMU commands?
- Are all requested VMs represented? Are absent values errors or explicit
  `null`/`UNKNOWN`, never silent zeroes?
- Are RSS, PSS, shared/private mappings, template page-cache bounds, cgroup
  accounting, and excluded kernel/KVM memory labelled separately?
- Are clone count, vCPU count, divergence fraction, and test-region size clear?
- Do the independent raw-log recount and summary agree?
- Are trace pauses, workload differences, sequential sample order, and low
  sample count disclosed?
- Are guest identity, disk backing, QMP endpoint, and sibling-survival checks
  present before claiming a VM is independent?

