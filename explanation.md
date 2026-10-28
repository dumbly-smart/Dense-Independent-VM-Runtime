# DenseVM explained from the beginning

This guide starts with what a virtual machine is and builds up to DenseVM's
RAM, disk, migration, identity, and measurement details. It assumes no prior
knowledge of Linux virtualization. The longer system map is
[`architecture.md`](architecture.md); measurement results are indexed in
[`reports.md`](reports.md).

## 1. The basic idea

A physical computer has CPU cores, memory chips, storage, and network devices.
An operating system controls those parts and runs programs.

A virtual machine (VM) is a computer simulated in software. It has virtual
CPUs, virtual RAM, virtual disks, and virtual devices. A guest operating system
runs inside the VM and behaves as if it owns a computer. The real computer
underneath is called the host; the OS inside the VM is called the guest.

If you start ten ordinary VMs, each gets its own virtual machine state. The
simple approach may require ten RAM copies and ten complete disk copies even
when all ten guests started from the same Linux installation. DenseVM asks:

> If the VMs start from the same state, can they initially use the same
> physical pages and disk blocks, then get private copies only when they write?

The answer is “yes for the tested mechanism,” with limits. The guests remain
separate QEMU VMs; only unchanged, safe-to-share backing data is common.

## 2. Host, guest, hypervisor, and QEMU

- **Host:** the real Linux machine running DenseVM and QEMU.
- **Guest:** the Linux OS running inside one VM.
- **Hypervisor:** software that creates and runs VMs. Linux's KVM subsystem
  provides hardware virtualization support; QEMU provides the virtual machine
  model, virtual devices, process, migration, and control interfaces.
- **QEMU process:** one OS process per guest in this project. It has its own
  file descriptors, QMP socket, serial socket, device objects, vCPU objects,
  and private process state.
- **KVM:** a Linux kernel interface that lets QEMU ask the CPU's hardware
  virtualization feature to execute guest instructions. KVM is not a second
  guest and is not a mechanism for merging CPU computations.
- **TCG:** QEMU's software translator. It translates guest instructions into
  host instructions. DenseVM uses it only as an optional comparison track; the
  main dense-VM path is KVM.

A process boundary matters. If QEMU A crashes, the OS can kill process A while
QEMU B remains alive. Separate processes also make CPU and device state
independent by construction, though a research test should still verify the
guest-visible properties it claims.

## 3. What is “state” inside a VM?

A running VM has more than RAM:

1. **RAM contents:** guest memory, including kernel, processes, page cache,
   stacks, and buffers.
2. **CPU state:** registers, control registers, instruction pointer, flags,
   model-specific registers (MSRs), and interrupt state for each virtual CPU.
3. **Device state:** virtual disk controller, network card, timers, interrupt
   controller, serial port, chipset, and other devices.
4. **Disk state:** filesystem and application blocks stored on virtual disks.
5. **Identity state:** machine-id, hostname, MAC/IP and DHCP identity, SSH
   keys, random-number-generator state, and application instance IDs.
6. **Time state:** virtual clock and timers, which can differ from wall time
   if the VM is paused.

Copying RAM alone does not recreate a running computer. DenseVM asks QEMU to
save and restore the non-RAM machine/device state through QEMU migration. It
does not invent its own serializer for QEMU's private data structures.

## 4. Why starting from a common template helps

Suppose three guests all start with identical page `P` at guest-physical page
number 100:

```text
one physical host page P
   ├── VM A reads P
   ├── VM B reads P
   └── VM C reads P
```

All three guests initially see the same bytes. That is safe because none has
changed them. If A writes to the page, it must not alter what B or C sees. The
host memory manager handles that with **copy-on-write**:

```text
Before A writes:      After A writes:

       P                    P
     / | \                 / \
    A  B  C               B   C
                           \
                            P_A
                             |
                             A
```

The first write faults on A's private mapping. The kernel allocates a new
physical page, copies the old contents, and lets A write the new page. B and C
continue to use the original `P`. This is not a message channel: later writes
are private.

The same principle is used for disks, but with a different mechanism. qcow2
backing files provide block-level overlays rather than RAM page faults.

## 5. Guest addresses and host pages

The guest thinks it has a continuous range of physical RAM called **guest
physical memory**. The host stores that data in **host virtual memory**, which
maps to real physical pages in RAM. The CPU and kernel translate addresses
through several layers:

```text
guest program address
      ↓ guest page tables
guest physical address
      ↓ KVM's second-level translation (EPT/NPT)
host virtual address
      ↓ host page tables
host physical page in RAM
```

For a file-backed template, QEMU maps the RAM file into the QEMU process. The
host's page cache can supply the same resident file page to multiple private
mappings. A write to a private mapping creates a private anonymous page for
that process. Guest-visible addresses do not change; only the host's mapping
and backing page change.

## 6. The RAM file and QEMU's memory settings

The source/template QEMU uses a file-backed memory backend with `share=on`
while it is creating the template. It is the writer during this phase. After
the guest reaches a checkpoint and QEMU is stopped, DenseVM freezes the RAM
file and does not modify it while clones use it.

Each clone opens the same RAM file with settings conceptually equivalent to:

```text
mem-path=<template RAM file>
share=off
readonly=on
rom=off
```

These settings mean:

- `mem-path` names the common template RAM backing file.
- `share=off` requests private mapping semantics for the clone.
- `readonly=on` prevents QEMU from modifying the template backing file.
- `rom=off` keeps the region from being treated as read-only guest ROM.
- The guest still sees writable RAM. A write goes into the clone's private
  copy-on-write page.

The key host-side proof is that `/proc/<pid>/smaps` reports the matching clone
mapping as writable and private (`rw-p`), and its private-dirty pages grow when
that guest changes memory. If the mapping were shared writable, one guest
could corrupt every sibling. DenseVM fails closed when the expected mapping is
not present or has the wrong permissions.

## 7. Why QEMU migration is needed

Imagine stopping a running VM and only keeping its RAM bytes. The guest CPU
would not know which instruction to execute next, devices would not know
whether an interrupt was pending, and the virtual disk controller would lose
its queue state. QEMU migration records those mutable machine details in a
QEMU-supported format.

DenseVM's basic flow is:

1. Start one source QEMU using the chosen machine type, CPU model, RAM size,
   vCPU count, and devices.
2. Wait for the guest to reach its `READY` checkpoint.
3. Use QMP to stop the VM and verify that QEMU reports it paused.
4. Enable the `x-ignore-shared` migration capability when the installed QEMU
   supports it. This asks QEMU to omit RAM already represented by the shared
   RAM file.
5. Migrate the non-RAM machine/device state to a file.
6. Stop the source process and protect the RAM/base disk as immutable inputs.
7. Start a separate destination QEMU for each clone with a matching machine
   configuration and private RAM mapping.
8. Ask QEMU to receive the saved migration state; wait for migration
   completion, then resume the guest.
9. Check guest responses and isolation.

`x-ignore-shared` is experimental. Availability is checked rather than
assumed. A small migration file is not proof that migration worked; the guest
must actually resume and pass the checks.

## 8. CPU and device independence

Each clone runs in a different QEMU process and gets its own vCPU objects and
virtual devices. DenseVM does not try to share mutable registers or virtual
device registers. A two-vCPU probe uses `-smp 2` for every conventional
baseline, the template, and each incoming clone, so migration's source and
destination topologies agree.

The current vCPU-count option accepts 1 through 4. Two-vCPU restore passed two
live runs. Existing HMP register capture shows CPU#0 only, so those results
prove a two-vCPU configuration restored and ran, but do not directly compare
vCPU1 registers between siblings. Broader MSR/APIC/interrupt semantics remain
partially measured.

## 9. Private disks with qcow2 overlays

A qcow2 image can name another qcow2 image as its backing file. Reads first
look for a changed cluster in the overlay and otherwise fall through to the
base. Writes allocate changed data in that guest's overlay:

```text
immutable base.qcow2
   ├── A.qcow2: A's changed clusters
   ├── B.qcow2: B's changed clusters
   └── C.qcow2: C's changed clusters
```

An overlay begins small even though its virtual capacity matches the guest
disk. It grows as the guest writes. DenseVM checks that the overlay's backing
path and format point to the intended base. Each QEMU process gets a separate
block device and separate overlay.

This keeps unchanged filesystem blocks common without sharing mutable disk
writes. Deleting or moving a base can break every child, so template manifests
and qcow2 backing-chain validation matter.

## 10. Identity is not automatically copied safely

A clone begins from a copy of the template's memory and device state. That can
copy values that should be unique:

- MAC address and IP configuration;
- DHCP client identifier and lease data;
- `/etc/machine-id` and hostname;
- SSH host private keys;
- kernel RNG state and already-initialized userspace random generators;
- application UUIDs, database IDs, and service credentials.

DenseVM generates fresh host-side per-clone identifiers. In the tested guest
profile, a private entropy payload is delivered and the kernel RNG is reseeded
before network setup. The Alpine identity profile rotates SSH keys and uses a
machine-id-derived DHCP option 61. A VMGenID device lets a capable guest
kernel notice that it was forked from a VM generation; this is a signal, not a
guarantee that every application rotates its identity.

The order matters: prepare unique identity first, then attach the NIC. Distinct
MAC addresses alone do not make machine-id, DHCP, SSH, or application state
unique. Evidence covers the tested Alpine profile, not every distribution.

## 11. QMP, serial, and lifecycle

QEMU exposes a JSON control protocol called **QMP**. DenseVM gives every
process a separate Unix-domain QMP socket. It uses QMP commands to stop,
resume, query status, configure migration, restore state, inspect identity,
and quit. It verifies transitions instead of assuming that a successful
command reply means the guest changed state.

The virtual serial console is a separate per-VM socket. DenseVM uses it for
readiness markers and a small guest-agent protocol: RAM reads/writes, disk
reads/writes, identity setup, workload commands, and guest status. Serial is
not used to infer QEMU process state.

Persistent clone records track process IDs and process fingerprints. The
fingerprints include the host boot ID and process start-time ticks, reducing
the chance that PID reuse looks like a still-running VM. Stop waits for QMP
closure and process exit. Reconcile handles crashes conservatively. Delete
refuses live/ambiguous clones and only removes that clone's own artifacts.

The read-only `template gc-plan` command checks that every discovered
generation is integrity-verified before suggesting candidates. Explicit
`--keep` IDs and clones recorded as `starting` or `running` are roots; each
root's parent generations are roots too because a descendant may need those
artifacts. Unmarked generations are candidate orphans, with leaf candidates
listed for a hypothetical deletion order. A duplicate ID, bad hash, missing
in-catalog parent, cycle, empty catalog, or active clone that points outside
the catalog blocks all candidates. The command only prints a plan: it does
not delete files, detach backing disks, acquire a catalog-wide lock, or
prevent files changing after validation. Run it against a quiescent catalog;
no destructive collector is implemented.

## 12. What the memory numbers mean

- **RSS:** all resident pages mapped by one process. If three processes share
  a page, each RSS may count it, so adding RSS overstates physical use.
- **PSS:** shared resident pages are divided among the processes mapping them.
  Summed PSS is useful for process comparisons but misses host allocations
  outside those processes.
- **USS:** commonly means private resident memory (private clean plus private
  dirty). It is an estimate derived from process mappings, not a separate
  kernel counter.
- **Shared clean/dirty:** resident pages reported as shared mappings. These
  totals are per-process views; summing them counts the same physical page
  multiple times.
- **Private clean/dirty:** pages private to one mapping/process. Growth in the
  template RAM mapping's private-dirty count is direct evidence of RAM COW.
- **Page cache:** cached file data may remain in RAM after a process exits or
  without being fully included in any process's PSS.
- **Cgroup `memory.current`:** a cgroup charge that can include some kernel,
  cache, and shmem memory. It is useful but not guaranteed to include every
  KVM/host allocation.

Therefore DenseVM does not say “N 2-GiB guests use 2 GiB” based on guest
configuration. It reports the actual process and cgroup observations, then
names missing costs such as kernel page tables and KVM metadata.

## 13. Template COW versus KSM

These are different:

- **Template COW:** clones intentionally start by mapping one common immutable
  RAM file. Sharing exists immediately, before clone pages are compared.
- **KSM:** Linux scans pages from separate memory areas and merges pages whose
  contents match. Sharing is discovered later, consumes scan CPU, and may
  create content/timing side channels.

DenseVM's primary experiment is template COW. KSM is an optional secondary
layer and has no positive measurement on the current host because the user
cannot change its host-global setting.

## 14. Divergence and density

Let `T` be the shared template footprint, `D_i` the private RAM/disk changes
for clone `i`, and `Q_i` the per-process CPU/device/QEMU overhead. The rough
cost model is:

```text
DenseVM cost ≈ T + Σ D_i + Σ Q_i + unmeasured host/kernel costs
```

The conventional model is closer to:

```text
ordinary VM cost ≈ Σ (full guest RAM + full guest disk + Q_i)
```

As clones write more, `D_i` grows. At 100% divergence of the tested 32 MiB
region, each clone owns almost all of that region privately; this says nothing
about untouched guest RAM. If guests become fully different, COW's benefit
shrinks. Density must be graphed against controlled divergence, not advertised
as a fixed factor.

## 15. KVM, CPU work, and TCG

KVM runs guest instructions on the host CPU using hardware virtualization.
If VM A and VM B both perform a million arithmetic operations, the host still
executes both workloads. Scheduling an idle VM, pausing it, or sharing its RAM
can reduce resource use, but does not remove active CPU instructions.

TCG translates guest instruction blocks into host code. DenseVM's TCG patches
measure translation starts, cumulative generated host-code bytes, and elapsed
translation wall duration. They do not measure current cache residency, and
the duration is not CPU time. Sharing generated TCG blocks between processes
is deferred because code can depend on guest address, CPU mode, helpers,
memory mappings, and invalidation state.

## 16. Parking and time semantics

An explicitly parked guest is QMP-paused. It consumes very little guest
execution CPU while paused, but it is not equivalent to a computer that kept
running. Guest time, timer delivery, RTC alarms, scheduling, and network
response can change. DenseVM records parking as an optional research
optimization and documents its effects. It does not automatically pause a
guest merely because it appears idle.

## 17. How to run the basic experiments

First inspect the host:

```sh
cargo run -- doctor
```

Compare ordinary QEMU guests:

```sh
cargo run -- benchmark baseline --out /tmp/base-2 --count 2 --accel kvm
```

Run the two-clone COW probe:

```sh
cargo run -- probe --out /tmp/cow-2 --count 2 --skip-guest-oom
```

Run two virtual CPUs per guest:

```sh
cargo run -- probe --out /tmp/cow-2-vcpu2 --count 2 --vcpus 2 --skip-guest-oom
```

Every output directory must be new. It contains raw JSON, guest artifacts,
QEMU logs, RAM/state files, and disk images appropriate to that experiment.
Some outputs are large; check available storage before running many samples.
The preflight may refuse unsafe host-memory budgets. Do not bypass it by
pretending virtual RAM is physical RAM.

## 18. What is still unknown

The repository intentionally leaves several questions open:

- positive KSM benefit and scan CPU cost on a host where KSM can be enabled;
- complete host physical memory including every KVM and kernel allocation;
- independent CPU state for vCPU1 and broader MSR/APIC/interrupt behavior;
- ordinary distro behavior beyond the tested Alpine identity profile;
- precise wall-clock, RTC alarm, and interrupt behavior during QMP parking;
- broad QEMU version, machine type, CPU model, and device compatibility;
- destructive hierarchical-template garbage collection and power-loss recovery;
- density when every guest's full RAM and disk diverge.

These are limits on what the saved experiments demonstrate, not implied
successes. See [`progress.md`](progress.md) for the current status and
[`reports.md`](reports.md) for the evidence paths.

