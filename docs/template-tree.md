# Two-generation template-tree experiment

`experiments/template-tree/probe.py` tests whether a diverged private clone can
be materialized as a second immutable RAM and disk template using QEMU's
migration machinery. It consumes a completed DenseVM probe output, creates one
parent clone, writes one RAM test region and one disk file, migrates that
stopped clone into a new QEMU process backed by a fresh writable
`memory-backend-file`, saves the child's non-RAM VMState, then restores two
descendants from the new RAM file and disk layer.

The experiment keeps the two migration roles distinct:

1. The parent clone is restored with `x-ignore-shared=on`, because its unchanged
   RAM comes from its parent template.
2. Before exporting the parent as a new generation, the experiment turns
   `x-ignore-shared` off. This transfer must carry the parent's complete guest
   RAM, including pages private to that clone.
3. The child template is restored into a fresh file-backed RAM image with
   `share=on,readonly=off`. After it is stopped, `x-ignore-shared=on` writes a
   compact machine/device state while leaving the now-complete shared RAM in
   that file.
4. Descendants map the child RAM file privately and read it with
   `share=off,readonly=on,rom=off`. Their disk overlays use the parent's
   read-only qcow2 overlay as backing, which itself points to the original
   immutable base.

QEMU's migration documentation describes migration as saving and restoring
device state and requires compatible source and destination machine
configurations. DenseVM records and reuses the same machine type and device
graph for this proof. The experiment is therefore a measured migration path,
not hand-written serialization of QEMU state. See the [QEMU 11.1.2 migration
framework documentation](https://qemu.readthedocs.io/en/v11.1.2/devel/migration/main.html).

## Reproduce

Build a regular template with `densevm probe`, then pass that output directory
to the experiment. It refuses an existing output path, less than 2 GiB of
available host memory, or less than 1 GiB of free disk space. Use a disk-backed
filesystem for both input and output. The experiment builds in a short hidden
staging directory and atomically renames it to the requested output path only
after all checks pass. A caught error removes the staging directory. A hard
process or host crash can leave an orphan `.dvm-*` staging directory, but does
not publish it at the requested path; automatic orphan recovery is not
implemented.

```sh
cargo build
cargo run -- probe --count 2 --skip-guest-oom --out /path/to/root-template
python3 experiments/template-tree/probe.py \
  --template-dir /path/to/root-template \
  --out /path/to/new-tree-experiment
```

The experiment writes its QEMU logs, child RAM file, child machine-state file,
qcow2 overlays, and a machine-readable `result.json` into the new output
directory. It checks input template hashes before and after the run, verifies
the qcow2 backing chain, samples the exact child-RAM VMA from each QEMU
process's `smaps`, and fails unless the descendants initially map resident
shared pages and A's RAM write increases A's private dirty pages. A successful
run also writes `generation.json` (schema version 2) with hashes for the child
RAM/state/disk artifacts and the parent result, RAM, machine-state, and base
disk, plus the QEMU version, source QEMU device command, and qcow2 backing
path. The generation ID is a SHA-256 content address over the child RAM,
machine-state, and disk hashes. `result.json` records the manifest hash. The
parent path is absolute, so the manifest is provenance for this experiment
and is not yet a relocatable template package. Run 3 preserves the prior v1
manifest; run 4 demonstrates the v2 content-addressed schema.

To check a complete experiment directory before reusing its child template,
validate the manifest beside the artifacts:

```sh
python3 experiments/template-tree/probe.py \
  --validate-manifest /path/to/new-tree-experiment/generation.json
```

The Python validator checks the child artifact hashes, the recorded parent
artifacts, the backing-image relationship, and the v2 content-derived
generation ID. DenseVM exposes the same read-only integrity check:

```sh
cargo run -- validate template /path/to/new-tree-experiment/generation.json
```

The Rust validator uses the host `sha256sum` utility. The small raw evidence
copies under `results/` intentionally omit the large RAM and qcow2 artifacts,
so validate the original experiment directory.

The Rust runtime can print the manifest's metadata without running the Python
experiment:

```sh
cargo run -- inspect template /path/to/new-tree-experiment/generation.json
```

That output is marked `metadata-only` with `integrity_verified: false`. Use the
validator above when artifact integrity must be checked.

## Measurements

Two repeated runs on QEMU 11.1.2 / Linux 7.2.11 used a synthetic 256 MiB KVM
template with a 64 MiB virtual disk and two descendants. Both runs observed
parent RAM and disk markers (`73`) after child-template creation. Descendant A
then changed both markers to `91`; descendant B continued reading `73` for
both RAM and disk. The root template's RAM, base disk, and machine-state hashes
were unchanged.

| Measurement | Run 1 | Run 2 |
| --- | ---: | ---: |
| Descendant A RAM-map PSS before → after A's write | 9,220 → 42,942 KiB | 9,154 → 44,688 KiB |
| Descendant B RAM-map PSS before → after A's write | 8,900 → 9,854 KiB | 8,898 → 9,886 KiB |
| A private dirty RAM-map pages before → after | 1,260 → 34,268 KiB | 1,192 → 36,104 KiB |
| B private dirty RAM-map pages before → after A's write | 940 → 1,180 KiB | 936 → 1,308 KiB |
| Child RAM file allocated storage | 159,383,552 bytes | recorded in raw result |
| Parent qcow2 overlay allocated storage | 331,776 bytes | recorded in raw result |

The shared dirty page counts in each process's `smaps` describe that process's
mapping; do not add them as if they were disjoint physical pages. PSS provides
the per-process proportional measurement. The QEMU migration statistics show
approximately 136.7 MB transferred when materializing the parent generation
and about 310 KB for the child's non-RAM machine-state snapshot. `qemu-img
check` passed on both descendant overlays in the first successful run; each
overlay's backing chain resolved through the parent overlay to the original
base image.

Each generation also received a different machine-id and hostname after
reseed. This synthetic profile has no SSH server, NIC, or DHCP client, so
identity behavior for those services was not exercised in this tree test.

Raw evidence: [`two-generation-run1.json`](../results/template-tree/2026-10-25/two-generation-run1.json),
[`two-generation-run2.json`](../results/template-tree/2026-10-25/two-generation-run2.json),
and [`two-generation-run3.json`](../results/template-tree/2026-10-25/two-generation-run3.json).
The third run's hashed generation manifest is also saved as
[`two-generation-run3-generation.json`](../results/template-tree/2026-10-25/two-generation-run3-generation.json).
The v2 content-addressed fourth run is recorded in
[`two-generation-run4.json`](../results/template-tree/2026-10-25/two-generation-run4.json)
and [`two-generation-run4-generation.json`](../results/template-tree/2026-10-25/two-generation-run4-generation.json).
Run 5 exercises atomic publication and post-publication manifest validation:
[`two-generation-run5.json`](../results/template-tree/2026-10-25/two-generation-run5.json)
and [`two-generation-run5-generation.json`](../results/template-tree/2026-10-25/two-generation-run5-generation.json).

## Scope and remaining work

This verifies one parent-to-child materialization and two independent
descendants for the tested synthetic guest and QEMU machine. The v2 generation
manifest is experimental provenance, not a persistent Template Manager. It
does not provide artifact relocation, garbage collection or orphan recovery,
durable publication after power loss, migration compatibility checks across
QEMU versions, network identity behavior across generations, or large
multi-level tree benchmarks.
It does not establish that every machine/device configuration supports the
same migration sequence. Those require separate failure and compatibility
tests before hierarchical templates are exposed as a general runtime feature.
