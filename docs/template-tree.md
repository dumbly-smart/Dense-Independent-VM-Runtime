# Hierarchical template-tree experiment

`experiments/template-tree/probe.py` tests whether a diverged private clone can
be materialized as a child immutable RAM and disk template using QEMU's
migration machinery. It accepts either a completed DenseVM probe output or a
previously generated template directory. It creates one parent clone, writes
one RAM test region and one disk file, migrates that stopped clone into a new
QEMU process backed by a fresh writable
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
process or host crash can leave a marked `.dvm-*` staging directory, but does
not publish it at the requested path. Recovery defaults to listing candidates;
it removes only marked directories after acquiring the publication lock and
checking same-user process command lines and memory maps for references. Active
stages and unrecognized `.dvm-*` paths are preserved. The stable hidden lock
file remains in the parent directory so concurrent publication and recovery
cannot race.

```sh
cargo build
cargo run -- probe --count 2 --skip-guest-oom --out /path/to/root-template
python3 experiments/template-tree/probe.py \
  --template-dir /path/to/root-template \
  --out /path/to/new-tree-experiment
# Later generations can use the prior generation directory as their input.
python3 experiments/template-tree/probe.py \
  --template-dir /path/to/new-tree-experiment \
  --out /path/to/next-generation
```

After an interrupted process, inspect possible orphan stages without changing
them, then explicitly remove safe candidates if appropriate:

```sh
python3 experiments/template-tree/probe.py --recover-staging /path/to/output-parent
python3 experiments/template-tree/probe.py --recover-staging /path/to/output-parent --remove-staging
```

The experiment writes its QEMU logs, child RAM file, child machine-state file,
qcow2 overlays, and a machine-readable `result.json` into the new output
directory. It checks input template hashes before and after the run, verifies
the qcow2 backing chain, samples the exact child-RAM VMA from each QEMU
process's `smaps`, and fails unless the descendants initially map resident
shared pages and A's RAM write increases A's private dirty pages. Earlier
successful runs wrote `generation.json` (schema versions 1-3) with hashes and actual
filenames for the parent result, RAM, machine state, and disk. This lets a
generation become the next generation's parent even when its artifacts are
named `child-memory.img`, `child-machine-state`, and `parent.qcow2`. The
generation ID is a SHA-256 content address over the child RAM, machine-state,
and disk hashes. `result.json` records the manifest hash. New runs write schema
version 4, with parent directories and qcow2 backing references relative to the
child generation. This makes a v4 dependency tree relocatable as a unit when
its relative directory structure is preserved. Older v1-v3 manifests remain
readable and retain their absolute-path behavior; a v4 child of a legacy
generation cannot repair absolute paths embedded in the older files.

To check a complete experiment directory before reusing its child template,
validate the manifest beside the artifacts:

```sh
python3 experiments/template-tree/probe.py \
  --validate-manifest /path/to/new-tree-experiment/generation.json
```

The Python validator checks the child artifact hashes, the recorded parent
artifacts, the backing-image relationship, and the content-derived generation
ID across schema versions 1-4. DenseVM exposes the same read-only integrity
check:

```sh
cargo run -- validate template /path/to/new-tree-experiment/generation.json
```

The Rust validator uses the host `sha256sum` utility. The small raw evidence
copies under `results/` intentionally omit the large RAM and qcow2 artifacts,
so validate the original experiment directory. Both validators check v3/v4
manifests against the parent generation's actual artifact names. V4 resolves
relative paths from the manifest directory.

The Rust runtime can print the manifest's metadata without running the Python
experiment:

```sh
cargo run -- inspect template /path/to/new-tree-experiment/generation.json
```

That output is marked `metadata-only` with `integrity_verified: false`. Use the
validator above when artifact integrity must be checked.

To discover nested generation manifests and connect parents found inside the
same directory tree:

```sh
cargo run -- template tree /path/to/template-store
```

The catalog walks directories without following symlinks and matches a
generation's resolved parent directory to another manifest directory. Parents
outside the catalog are shown with `parent_generation_id: null`. The catalog
is metadata-only and does not verify artifact contents or maintain a separate
index; each `generation.json` remains the source of truth.

Before considering removal of unreferenced generations, ask DenseVM for a
non-destructive reachability plan:

```sh
cargo run -- template gc-plan /path/to/template-store \
  --state-dir /var/lib/densevm \
  --keep sha256:GENERATION_ID
```

The state directory must already exist. DenseVM reads clone records without
creating the registry or changing its permissions. Every generation and its
manifest-referenced artifacts must pass integrity validation before any
candidates are reported. Generations referenced by `starting` or `running`
clones and repeated `--keep` IDs are protected, along with their discovered
ancestors. An empty or invalid catalog, duplicate IDs, malformed or cyclic
parent links, or an active clone that points to an unknown generation blocks
all candidates. A planned candidate includes its ID, directory, manifest,
parent ID, and leaf flag; leaf IDs are listed separately as a possible
child-first removal order. The report lists each active clone's registry state
alongside its generation reference.

To remove one verified, unreferenced leaf generation, specify its ID and the
explicit `--apply` flag:

```sh
cargo run -- template gc /path/to/template-store \
  --state-dir /var/lib/densevm \
  --generation sha256:GENERATION_ID \
  --keep sha256:GENERATION_TO_RETAIN \
  --apply
```

Collection recomputes the catalog plan while holding the publication lock,
then takes an exclusive lock in that generation directory. DenseVM clone
startup takes a shared generation lock while opening and restoring the
artifacts. This prevents cooperating clone starts from racing the unlink. The
command removes only the verified manifest-listed RAM, machine-state, disk,
and `generation.json` files for a candidate that is currently an unreferenced
leaf. It preserves the generation directory, logs, result files, and every
other unrecognized entry; parent backing artifacts are untouched. Repeated `--keep` values are applied again during the delete-time reachability
check; pass the same keep IDs used when reviewing the plan. Remove a tree from
leaves inward by running the command once per leaf, using the current plan each
time.

The lock only coordinates DenseVM's publisher, collector, and clone-start
paths. It cannot detect or serialize external QEMU processes or unrelated
references outside the supplied clone registry/catalog. An interrupted unlink
has no recovery journal and may leave an invalid manifest/artifact set; the
validator will fail closed afterward and manual recovery may be required. A
disposable CLI smoke using hard-linked copies of generated artifacts removed
the four child files while preserving the source and parent hash:
[`template-gc-cli-smoke.json`](../results/kvm-recovery-2026-10-30/template-gc-cli-smoke.json).
This is experimental collection, not crash-safe storage management.

This command only reports candidates; it does not remove files or check
references outside the clone registry and catalog. It now takes the same
nonblocking `.densevm-template-tree.lock` used by the Python publisher and
staging recovery while it walks and hashes the catalog. If another cooperating
publication/recovery holds the lock, planning fails immediately. Creating the
lock for a catalog that has never been published by the experiment tool leaves
that mode-0600 lock file in the catalog root. The lock is advisory, so external
writers that do not use it are not serialized. Treat the plan as an audit aid,
not authorization to delete a directory. A blocked integrity plan still
prints its JSON blockers and exits nonzero, so automation cannot mistake it
for a usable candidate plan.

### Relocation proof

Three successive schema-v4 generations were created from a 256 MiB synthetic
KVM template. The first two generations and their root source were copied to a
new directory while preserving the tree layout. Python and Rust validated the
manifests there, and `qemu-img info --backing-chain` opened the full disk chain.
A third generation then booted and restored from the moved files, passed the
RAM COW and disk-isolation checks, and emitted another v4 manifest. The full
record, including hashes and per-clone RAM mapping samples, is
[`results/template-tree/2026-10-26/relocation-proof.json`](../results/template-tree/2026-10-26/relocation-proof.json).
This proves relocation for that complete tree and QEMU version; it does not
support moving only a subset of the dependencies or establish other device and
distribution combinations.

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

### Recursive generation proof

Two additional live runs used generated templates as input. The first created
a v3 generation from a v2 parent; the second created another v3 generation
from that v3 parent. Both resumed from inherited `RAM 73` and `DISK 73`
markers. In each run A changed both to `91`, while B retained `73`. The full
qcow2 chain was checked at every generation, including all previous backing
layers. DenseVM's catalog found five saved manifests and resolved the three
generated nodes from the latest output back to the original root template.

| Measurement | Recursive run 1 | Recursive run 2 |
| --- | ---: | ---: |
| Descendant A RAM-map PSS before → after write | 11,998 → 44,512 KiB | 10,354 → 43,336 KiB |
| Descendant B RAM-map PSS before → after A's write | 9,150 → 10,352 KiB | 10,218 → 10,432 KiB |
| A private dirty pages before → after | 3,976 → 36,084 KiB | 1,052 → 33,920 KiB |
| B private dirty pages before → after A's write | 1,128 → 1,896 KiB | 1,016 → 1,032 KiB |

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
Recursive runs and the catalog snapshot are under
[`results/template-tree/2026-10-26/`](../results/template-tree/2026-10-26/).
[`root-compatibility-run1.json`](../results/template-tree/2026-10-26/root-compatibility-run1.json)
also records the standard probe-output-to-v3 path after recursive support was
added.

## Scope and remaining work

This verifies repeated parent-to-child materializations and independent
descendants for the tested synthetic guest and QEMU machine. The schema-v4
generation manifest and Rust catalog are experimental management tools, not a
persistent Template Manager. The runtime `clone start/stop` path has also been
verified with eight clones from a relocated nested v4 generation; all inherited
parent state, isolated A-only RAM/disk writes, and stopped cleanly. See the
[`eight-clone persistent template-tree results`](../results/runtime-clone/2026-10-26/persistent-template-tree-run5/README.md).

The experiment now recovers marked orphan staging directories after a process
crash. This is not durable publication across power loss: artifact and parent
directory `fsync` ordering has not been implemented or tested. Process
reference detection covers same-user `/proc` command lines and memory maps; it
is not a general process supervisor or a persistent template manager. The
runtime now has explicit leaf-only garbage collection, but deletion is not
crash-safe and there is no managed catalog with durable recovery. Migration
compatibility across QEMU versions and large multi-level tree benchmarks remain
untested. These results also do not establish that every machine/device
configuration supports the same migration sequence. Those gaps need separate
failure and compatibility tests before hierarchical templates become a general
runtime feature.
