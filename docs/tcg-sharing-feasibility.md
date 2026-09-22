# TCG translation sharing feasibility

This is an analysis only. The working probe uses KVM and does not share CPU
execution or translated code.

QEMU TCG translates guest basic blocks into `TranslationBlock` objects owned by
a process's `TCGContext`. A block is selected by guest virtual or physical
program counter, CPU mode, privilege state, page permissions, and translation
flags. The generated host code also reaches process-local helper functions,
register state, and memory access paths. Those dependencies prevent directly
sharing a `TranslationBlock` pointer between independent QEMU processes.

Parts that can be identical across related guests include the decoded guest
instruction sequence and, in some configurations, immutable host machine code
for a block whose target addresses and helper ABI are stable. Parts that are
not safely identical include CPU flags and register values, MMU/TLB state,
guest physical address translations, privilege and exception paths, self
modifying code checks, I/O exits, breakpoints, plugins, and process-local TCG
contexts.

Blocks are invalidated when guest code is written, mappings or permissions
change, CPU mode changes, or a TCG flush is requested. Self-modifying code and
cross-page writes make an immutable shared cache difficult to validate. A
shared cache would also need an exact QEMU build, host code-generation ABI,
CPU feature set, target configuration, and invalidation protocol.

The likely safe research direction is a content-addressed cache of decoded
guest bytes and translation metadata, with each QEMU process materializing its
own process-local host code after validating the key. That can reduce repeated
translation work and disk or startup cost, but it does not remove per-process
execution state and may save little for short-lived or divergent guests.

No QEMU patch is justified yet. The KVM/template path must remain the primary
result; a TCG experiment should first measure translation CPU time and cache
size, then compare a process-local cache against a prototype immutable decoded
block cache. Any implementation must test self-modifying code, page aliasing,
interrupts, exceptions, CPU mode changes, and cache invalidation before making
semantic claims.
