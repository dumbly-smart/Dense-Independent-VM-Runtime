# TCG translation sharing feasibility

**Status: analysis only.** DenseVM currently runs KVM guests. KVM executes
guest instructions on the host CPU, so there is no QEMU TCG translation cache
to share in the current runtime. This document evaluates a separate,
future-facing TCG experiment. It does not propose changing the KVM design or
claim that translated code can eliminate guest CPU work.

The source and internals below are checked against QEMU 11.1.2, the version on
the measured host. QEMU's TCG documentation describes translation blocks
(TBs), CPU-state assumptions, direct block chaining, self-modifying-code
invalidation, and the system-mode software MMU. The multi-threaded TCG design
uses per-vCPU jump caches and TLBs alongside process-global TB lookup and
translation structures. Those structures are internal to a QEMU process;
QEMU does not document a cross-process TB cache.

## What a translation block contains

TCG translates a guest basic block into host instructions and keeps metadata
that lets QEMU find, execute, restore, and invalidate that code. The TB is not
just a portable decoding of guest instruction bytes. Its identity and generated
code depend on guest PC/address translation, CPU state that the translator
assumes stable during the block, the configured guest CPU, TCG options, host
architecture and ABI, QEMU build, and the process-local execution environment.

Some state remains live at run time. In QEMU's TCG intermediate representation,
the fixed `cpu_env` global points to that vCPU's `CPUArchState`. Generated code
and helpers use that per-VM architectural state and QEMU's memory-access path.
Consequently, two guests executing identical bytes still need independent
register state, memory mappings, interrupts, and device effects.

## Research questions

### 1. Which parts of a TB depend only on guest code?

The guest instruction bytes and some decode decisions can match when the
guests use the same instruction set, enabled CPU features, execution mode, and
relevant translation flags. The same byte sequence can therefore produce
similar intermediate operations.

The bytes alone do not identify a reusable TB. For example, x86 decoding and
optimizations depend on mode and CPU features; QEMU records state assumptions
in the TB and selects another TB when those assumptions do not match. An
instruction byte cache or decoded-IR cache is a possible research object. A
fully emitted TB is not safely identified by a hash of the guest bytes alone.

### 2. Which parts depend on CPU state?

The TB key records CPU state that translation treats as constant. A privilege
or execution-mode change can require a new TB. On x86, segment-base assumptions
can change which address arithmetic the translator emits. CPU features,
translation flags, and per-vCPU execution context also affect generated code
or its safe lookup.

The guest's changing general-purpose registers, control registers, MSRs,
interrupt state, and device state remain per VM. TCG uses `cpu_env` to access
the current `CPUArchState`; it does not turn those values into shared state.

### 3. Which parts depend on guest physical addresses?

For system emulation, QEMU's software MMU translates guest virtual addresses
and indexes TB lookup by guest physical address. The TB also retains guest PC
and page information used for lookup, linking, exception recovery, and
invalidation. Per-vCPU TLB entries resolve guest memory accesses against that
VM's mappings and memory regions. A guest physical address is therefore part
of QEMU's system-mode TB organization even when two clones contain identical
code bytes at different addresses.

A portable decoded representation could be keyed by content rather than
address, but each VM would still need to validate its own mapping, permissions,
CPU context, and data-access behavior before executing a local TB.

### 4. What invalidates a TB?

QEMU documents invalidation when translated guest code is modified. System
emulation uses software-MMU write protection and page-to-TB bookkeeping to
remove affected translations. QEMU also reverses direct TB-to-TB jump patches
and updates lookup caches when a TB becomes invalid. Mapping or page changes,
debug breakpoint changes, some CPU helpers, plugin changes, and a full TCG
buffer flush can also invalidate individual or broad sets of translations.

Any shared cache would have to preserve these rules per VM. One guest's code
write must not invalidate, mutate, or redirect another guest's TB, while both
must stop using a translation when their own code or translation context
changes.

### 5. Could an immutable translated-code cache be reused safely?

**Not as a raw `TranslationBlock` or host-code pointer across QEMU processes.**
TBs contain process-local metadata, jump patches, links, and host-code addresses.
Generated code can access a process's vCPU state, helpers, TLBs, and code-cache
structures. Sharing those pointers would couple clone state and invalidation,
violating the independent-VM requirement.

A content-addressed cache of guest bytes or a carefully defined decoded
intermediate representation is more plausible. Each QEMU process could
validate the cache key and generate its own host code and TB metadata. That
would save repeated decode or optimization only; it would still pay local
code-generation costs and execute every guest instruction independently.

Sharing emitted host code would require a new QEMU-supported representation
with relocations or indirection for all process-local addresses, plus an exact
compatibility key for the QEMU build, TCG backend, host architecture/ABI, guest
CPU model and features, translation flags, and code bytes. QEMU exposes no
supported interface for doing this today. This project should not mmap a
QEMU-generated code cache between processes and call it safe.

### 6. What correctness hazards exist?

The central hazards are stale code after self-modifying writes; code aliases
through different guest virtual or physical mappings; translation keys that
omit mode, privilege, CPU feature, or instrumentation state; races with
invalidation and direct-jump patching; and references to process-local CPU,
helper, TLB, or memory objects. Exception recovery also maps host instruction
locations back to guest instructions, so code movement or reuse needs matching
per-VM metadata. Plugins, breakpoints, interrupts, MMIO exits, and QEMU build
changes add more invalidation or code-generation dependencies.

The isolation invariant is strict: if VM A writes code or changes mappings,
VM B's execution and TB cache must remain correct without exposing A's state.
Any prototype would need adversarial tests for each listed case before
claiming correctness.

### 7. Would sharing save meaningful resources?

**Unknown; probably limited to workloads with substantial repeated translation.**
A decoded-code cache might reduce decode/optimization CPU time and repeated
translation metadata. A shared immutable representation might also reduce
storage for cached input or IR. It would not remove per-VM vCPU state, QEMU
device state, memory accesses, scheduling, or execution of guest instructions.
It may add lookup, validation, synchronization, and invalidation overhead.

The first experiment should measure TCG translation time, generated-code
bytes, TB counts, and startup time for ordinary independent TCG VMs. Then
compare a process-local decoded cache with a prototype immutable decoded-IR
cache. Measure active workload CPU separately from translation savings. If
translation is a small share of total cost, stop before attempting a cache.

## Decision

No QEMU patch or cross-process host-code cache is justified by the current
evidence. Keep this track deferred until the KVM/template measurements leave a
specific TCG research question. Any experiment must remain separate from the
KVM result and report its own correctness and performance evidence.

## Sources

- [QEMU 11.1.2 Translator Internals](https://qemu.readthedocs.io/en/v11.1.2/devel/tcg.html): CPU-state assumptions in TBs, direct block chaining, code invalidation, exception recovery, and physical TB indexing in system emulation.
- [QEMU 11.1.2 TCG Intermediate Representation](https://qemu.readthedocs.io/en/v11.1.2/devel/tcg-ops.html): `cpu_env`, TCG globals, and guest memory operations.
- [QEMU 11.1.2 Multi-threaded TCG](https://qemu.readthedocs.io/en/v11.1.2/devel/multi-thread-tcg.html): per-vCPU jump caches/TLBs, shared TB lookup structures, and invalidation bookkeeping.
- [QEMU 11.1.2 TCG Emulation overview](https://qemu.readthedocs.io/en/v11.1.2/devel/index-tcg.html): the TCG subsystem scope and its relationship to hardware-accelerated hypervisors.

These documents explain QEMU's implementation model. The conclusions about
cross-process reuse and expected savings are analysis for DenseVM, not a QEMU
guarantee or a measured result.
