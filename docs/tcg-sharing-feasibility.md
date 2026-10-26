# TCG translation sharing feasibility

**Status: analysis only.** DenseVM currently runs KVM guests. KVM executes
guest instructions on the host CPU, so there is no QEMU TCG translation cache
to share in the current runtime. This document evaluates a separate,
future-facing TCG experiment. It does not propose changing the KVM design or
claim that translated code can eliminate guest CPU work.

The host has `qemu-system-x86_64` 11.1.2 (`qemu-base` 11.1.2-1) and the
matching versioned TCG documentation installed under
`/usr/share/doc/qemu/qemu/devel`. A QEMU source tree or source package was not
installed locally. The findings below are therefore checked against the
version-matched upstream documentation, not a local source-code audit. That
documentation describes translation blocks (TBs), CPU-state assumptions,
direct block chaining, self-modifying-code invalidation, and the system-mode
software MMU. It also describes per-vCPU jump caches and TLBs alongside
process-wide TB lookup and translation structures. QEMU documents no supported
cross-process TB cache.

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

The first experiment should use ordinary independent TCG VMs and measure
translation time, generated-code bytes, TB counts, and startup time separately
from guest execution. The installed documentation does not establish a ready
QMP counter for all of these, so collecting them may require instrumentation
in a separate QEMU build. If repeated translation is material, first compare
the baseline with a process-local decoded cache; only then evaluate an
immutable cross-process decoded-IR representation. Measure active workload
CPU separately from translation savings. If translation is a small share of
total cost, stop before attempting a shared cache.

## Decision and next experiment gate

The KVM/template prerequisite for investigating this separate track is now
met. This analysis does not justify a QEMU patch or a cross-process host-code
cache: there is no supported interface for sharing emitted TBs, and the
correctness burden is much larger than sharing decoded guest code.

The next useful step is an instrumented TCG baseline using independent QEMU
processes. Use the same guest image and workload across VM counts, and measure
translation cost separately from guest execution. Proceed to cache experiments
only if repeated translation is a material measured cost, starting with a
process-local decoded cache. Keep the experiment separate from the KVM results.
The baseline CLI now supports `--accel tcg`; two TCG one-VM and two-VM runs,
and two KVM two-VM runs, are saved under `results/tcg/2026-10-26/`. They record
readiness time, process memory, idle CPU ticks, and active CPU ticks. The TCG
`max` CPU model differs from KVM's host CPU model, and two runs per
configuration are not a statistical comparison. Expected cache savings remain
unknown.

For bounded event counts, `--trace-translations` enables QEMU's
`translate_block` trace event and saves the trace log in the run directory.
This mode is limited to two VMs because the log grows with guest translation
activity. With stock QEMU, it reports counts observed through the active CPU
sample but no translation duration or generated-code size. Patched QEMU can
also report those metrics, as described below. Startup and CPU metrics from a
traced run are perturbed and must be compared only with other traced runs. Two
one-VM QEMU 11.1.2 samples recorded 108,834 and 109,377 events in
8,759,535-byte and 8,803,502-byte logs; raw results and logs are under
`results/tcg/2026-10-26/trace1/` and `trace1b/`.

The trace parser also reports distinct guest PC values and how often those
values appeared more than once. This is only a guest-PC reuse indicator: one
PC can produce multiple TBs when translation state differs, so the number is
not a unique-TB count or a count of retranslations. Across the two samples,
event counts were 108,834–109,377; distinct guest PCs were 92,654–92,677;
events at repeated PCs were 28,410–28,999 across 12,230–12,299 PCs; and the
maximum observed events for one PC was 9–10. This indicates repeated
translation events at some entry addresses, but it does not show whether the
same TB was recreated or how expensive those events were.

QEMU 11.1.2's local `-d help` lists `out_asm`, which prints generated host
assembly and labels each emitted block with its byte size. It is a possible
source for cumulative generated-code bytes, but it is highly verbose. A manual
probe produced a 395 MB text log and did not yield a guest-ready checkpoint
within its observation window. That probe yielded no accepted measurements. A
future disassembly-based measurement would need to bound disk use and show the
guest reaches the same checkpoint. Upstream QEMU's installed trace-event list
still has no direct translation-duration event; the separate patch below
provides one for the experiment.

The matching QEMU 11.1.2 source shows that `translate_block` fires before
`setjmp_gen_code`, while the generated size is stored in `tb->tc.size` after
code generation. `patches/qemu/0001-trace-tcg-generated-block-size.patch`
adds a completion trace event at that point, avoiding full assembly output.
The patch was built in an isolated x86_64-softmmu TCG build and run through
DenseVM's existing trace benchmark. The saved run at
`results/tcg/2026-10-26/codegen3/` recorded 109,238 translation starts,
109,218 completion events, and 34,171,065 cumulative generated host-code
bytes in a 17,577,176-byte trace log. DenseVM stopped the guest after the
active workload before reading the log; the parsed start and completion event
counts exactly match the frozen raw trace. The 20-event count difference is
not correlated to individual TBs and can include restart paths.

Three further runs with
`patches/qemu/0002-trace-tcg-translation-wall-duration.patch` measured elapsed
monotonic wall time around `setjmp_gen_code`. The samples recorded
108,907–109,285 completed blocks, 34,092,023–34,162,552 cumulative generated
bytes, and 1,155,076–1,173,435 microseconds cumulative wall duration
(10.57–10.77 microseconds per completed block on average; maximum event
duration 530–1,896 microseconds). This duration includes target translation and TCG
optimization/code emission, but excludes later code-cache publication. It is
wall time, not thread CPU time, and can include host scheduling delay. The
three sequential samples are directional measurements, not a statistical
result.
Only successful code-generation calls contribute duration; failed or restarted
attempts are omitted and remain visible only in the separate start/completion
event-count difference.

The trace runner now takes two additional QMP-frozen log snapshots: after the
guest reports `READY`, and after the one-second idle CPU sample just before
starting `cpu-burn`. It reports separate boot/readiness, post-ready idle, and
active-workload summaries, with byte offsets into the raw log. The final
snapshot is taken after the active workload completes. Phase counters are
recountable from those exact raw log byte ranges. These are QMP pause points,
not guest-instruction boundaries; pausing/resuming at both intermediate
boundaries perturbs the traced guest. The ready time is recorded before the
first diagnostic pause. With multiple guests, the boot/readiness segment for
each guest continues until all guests have emitted `READY` and the freeze loop
reaches that guest.

Three sequential patched-QEMU runs at `results/tcg/2026-10-27/phases1/`
through `phases3/` recorded the following ranges:

| Phase | Translation starts | Completed blocks | Cumulative generated bytes | Successful code-generation wall time |
| --- | ---: | ---: | ---: | ---: |
| Boot through readiness freeze | 107,795–108,304 | 107,775–108,284 | 33,724,574–33,900,046 | 1,225,067–1,343,860 µs |
| Post-ready idle through pre-active freeze | 189–517 | 189–517 | 65,660–153,949 | 3,824–5,863 µs |
| Active workload through final freeze | 711–763 | 711–763 | 226,886–244,054 | 8,920–18,365 µs |

In each run, phase start-event totals exactly equal the complete-log count,
and completed-block, generated-byte, and duration totals sum to their
full-trace values. An independent raw-log recount matched each phase range.
The three sequential samples are directional, not a statistical benchmark.
Readiness ranged from 8,145 to 8,797 ms and process PSS from 302,500 to
303,915 KiB. Tracing and diagnostic freezes perturb both measurements.
Three one-VM control runs with the same patched binary but tracing disabled
measured 8,331–9,036 ms to readiness and 296,296–301,462 KiB process PSS.
The traced runs measured 8,145–8,797 ms and 302,500–303,915 KiB respectively.
Readiness is captured before phase pauses; the PSS comparison includes trace
output and post-ready pauses. The overlapping ranges and sequential `n=3`
samples do not establish trace overhead or a startup benefit. Raw controls are
under `results/tcg/2026-10-27/untraced1/` through `untraced3/`.

A stock-QEMU trace run is also saved at
`results/tcg/2026-10-26/stockduration/`; it correctly reports the generated-code
and duration metrics as unavailable while preserving ordinary translation
counts. Reproduce the patched run using the QEMU v11.1.2 source and both
patches, then put the custom binary first in `PATH`; the exact build and run
commands are in `patches/qemu/README.md`. See the [versioned translate-all.c
source](https://github.com/qemu/qemu/blob/v11.1.2/accel/tcg/translate-all.c)
and [trace event definitions](https://github.com/qemu/qemu/blob/v11.1.2/accel/tcg/trace-events).

## Sources

- [QEMU 11.1.2 Translator Internals](https://qemu.readthedocs.io/en/v11.1.2/devel/tcg.html): CPU-state assumptions in TBs, direct block chaining, code invalidation, exception recovery, and physical TB indexing in system emulation.
- [QEMU 11.1.2 TCG Intermediate Representation](https://qemu.readthedocs.io/en/v11.1.2/devel/tcg-ops.html): `cpu_env`, TCG globals, and guest memory operations.
- [QEMU 11.1.2 Multi-threaded TCG](https://qemu.readthedocs.io/en/v11.1.2/devel/multi-thread-tcg.html): per-vCPU jump caches/TLBs, shared TB lookup structures, and invalidation bookkeeping.
- [QEMU 11.1.2 TCG Emulation overview](https://qemu.readthedocs.io/en/v11.1.2/devel/index-tcg.html): the TCG subsystem scope and its relationship to hardware-accelerated hypervisors.
- [QEMU 11.1.2 source tree](https://gitlab.com/qemu-project/qemu/-/tree/v11.1.2): upstream source for follow-up implementation-level inspection; it was not installed on the measured host.

These documents explain QEMU's implementation model. The conclusions about
cross-process reuse and expected savings are analysis for DenseVM, not a QEMU
guarantee or a measured result.
