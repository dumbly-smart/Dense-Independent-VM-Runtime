# DenseVM: first experiment design

## Research target

DenseVM asks whether separate QEMU/KVM processes can resume from one frozen
Linux machine state while sharing unchanged guest RAM and disk blocks. Milestone
1 is an experiment, not a density claim. A result is accepted only when two
running guests, isolation probes, and host memory measurements exist together.

The initial guest is a deliberately small Linux kernel and Rust init program.
It is a mechanism probe, not a booted general-purpose distribution with system
services or a login prompt. Passing this probe is necessary but does not by
itself satisfy the full Milestone 1 criteria for an ordinary Linux VM.

## Host and version lock

Inspected on 2026-09-22: QEMU/qemu-img 11.1.1, Linux
7.2.6-arch2-1, 4096-byte base pages, usable `/dev/kvm`, KSM compiled in but
`run=0`, 15 GiB host RAM (about 9.8 GiB available at inspection). The only
tracked file before this design was `README.md`; git has one initial commit.
No reusable guest disk image or installed QEMU source tree was found. The
installed QEMU 11.1.1 HTML documentation is under `/usr/share/doc/qemu/qemu`.

Local references: `system/vm-templating.html`, `system/images.html`,
`tools/qemu-img.html`, `interop/qemu-qmp-ref.html`, and `specs/vmgenid.html`.
The live executable also confirmed `memory-backend-file` properties and QMP
`query-migrate-capabilities` includes `x-ignore-shared` and `mapped-ram`.

Environment refresh, 2026-10-24: the active host is QEMU/qemu-img 11.1.2 on
Linux 7.2.11-arch1-1, 4096-byte pages, `/dev/kvm` accessible, and KSM `run=0`.
The read-only `densevm doctor` output is saved at
`results/2026-10-24/host-capabilities.json`. The original 2026-09-22 record
above remains as historical provenance.

## State model

* One QEMU process per VM. Each process owns vCPU, APIC, timer, virtio,
  serial, QMP, and block device state. QEMU migration serializes the mutable
  architectural and device state; DenseVM does not serialize QEMU internals.
* The source uses `memory-backend-file` with `share=on`. At a stopped
  checkpoint, its RAM file becomes immutable. `x-ignore-shared` must exclude
  that RAM from the machine-state migration stream. It is experimental, so
  startup probes for it and restore must be demonstrated before integration.
* Clone QEMU processes use the same exact RAM file with `readonly=on`,
  `rom=off`, `share=off`. This should create a writable private mapping and
  fault unchanged pages from the shared file cache. No process may write to
  the RAM file after freezing. The file must not be replaced, truncated, or
  removed while clones exist.
* A qcow2 base disk is immutable after the checkpoint. Every clone receives
  its own qcow2 overlay made with `qemu-img create -f qcow2 -b ... -B qcow2`.
  Backing paths and format are verified with `qemu-img info --backing-chain`.
* Source and destination must pin the same QEMU executable version, explicit
  machine type, CPU model, guest RAM size, device graph and boot artifacts.
  Changes to these are template compatibility failures.

## Minimal proof sequence

1. Compile a static Rust `/init`, assemble a small initramfs, boot the local
   Linux kernel under QEMU/KVM with a small file-backed RAM object and a qcow2
   disk. The guest reports a ready marker and accepts serial RAM/disk probes.
2. Wait for readiness, issue QMP `stop`, verify stopped state, enable
   `x-ignore-shared`, and migrate the stopped machine state to a file. Record
   hashes and sizes for RAM, disk and machine-state artifacts. Exit the source.
3. Make two thin disk overlays. Launch two separate clone QEMUs with separate
   QMP and serial sockets, unique MACs, private RAM mappings and the same
   machine-state file as incoming migration. Resume only after restore
   completion is observed.
4. Probe both guests before and after RAM and disk writes in A, then write B.
   Confirm no cross-clone data change. Kill A's QEMU process and confirm B
   still responds. Confirm source RAM file hash remains unchanged.
5. Sample `/proc/<pid>/smaps_rollup` and the exact RAM-file VMA from
   `/proc/<pid>/smaps` for RSS, PSS, shared and private pages. Compare two
   clones against two equivalent ordinary QEMU/KVM guests. Store raw JSON
   including QEMU command lines, PIDs, timestamps and environmental metadata.

The first probe uses at most 256 MiB per VM and a small disk, stays below the
host's available-memory budget, and does not alter global KSM settings.
Machine-state restore errors, guest non-response, or private-page measurements
that contradict COW are failures, not partial successes.

## Identity and semantics

The first probe hotplugs a new NIC with a distinct MAC into each restored
clone and assigns distinct QMP/serial sockets. It also writes and reads a
synthetic per-clone machine-id, hostname, and deterministic seed token through
the guest serial control path; this is an identity-divergence mechanism test,
not a cryptographic RNG proof. An initial run with NICs in the
template showed that migration restored the template MAC over clone launch
arguments; command-line MAC differences alone did not provide uniqueness.
It also presents a distinct vmgenid to each clone if migration preserves the
change; this must be checked in the guest before claiming it. A copied guest
machine-id, DHCP client ID, hostname, SSH host key, application ID, or RNG
state is **not** automatically fixed by QEMU. The synthetic probe has none of
those services; full Linux clones must run a guest identity-divergence protocol
before networking is enabled. Until that is verified, distribution guests
must not be exposed to a shared network. Separate MACs alone are insufficient.

QEMU documents VM templating's security alert and the same identity hazards.
`vmgenid` signals a generation change to a capable guest; it does not by
itself guarantee that Linux userspace rotates every identity or secret.

## Measurement and interpretation

RSS double-counts shared pages. Sum of clone PSS is the primary user-space
resident footprint; private clean/dirty and shared clean/dirty are reported
separately. The RAM-file VMA is identified by path in `smaps`, not guessed
from total RSS. File allocation is read from `stat`/`qemu-img info`, never
from virtual disk size. Kernel page tables, KVM metadata, page cache and QEMU
overhead must be reported or named as unmeasured; PSS alone is not the entire
physical cost. KSM stays disabled for the primary COW comparison. KVM does
not share translated instruction work.

The optional `mapped-ram` migration capability is distinct from the private
RAM mapping and is not required for the first proof. `x-ignore-shared` is
experimental in QEMU 11.1.2; failure here changes the design rather than
justifying a fabricated resume claim.

## Follow-on gates

The basic two-clone mechanism and Alpine service/login checkpoint have been
demonstrated. An opt-in Alpine identity profile now demonstrates OpenSSH host
key rotation, DHCP client identity, and the tested Linux kernel's VMGenID
reaction. Remaining gates are broader distribution coverage, device/register
and direct timer-interrupt tests, positive KSM measurements, and complete
kernel-memory accounting. KSM measurements are blocked by the current user's
permissions for host-global sysfs. Template trees and TCG sharing remain
deferred; explicit parking is measured but changes guest time.
