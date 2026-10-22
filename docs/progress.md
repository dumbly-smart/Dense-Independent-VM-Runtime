# Remaining research execution record

Plan: `docs/superpowers/plans/2026-10-22-densevm-remaining-research.md`.

## 2026-10-22

- Working branch: `research/remaining-runtime`; existing untracked plan retained.
- Graph discovery: graft inspected current APIs and probe callers. Codebase-memory MCP has no project indexed for this checkout, so exact source reads supplement graft.
- Initial baseline suite: 8 tests passed, 1 hardware integration test ignored.
- Fresh unmodified probe on QEMU 11.1.2 / Linux 7.2.11 passed RAM/disk isolation, identity checks, and explicit parking. Artifacts: `/tmp/densevm-20261022-initial`.
- Part 1: `doctor` reports kernel/page/tick settings, KVM accessibility, QMP migration capabilities, memory backend properties, machine compatibility, and KSM availability. Probe metadata includes the report and verifies KVM is actually enabled. Unsupported template mechanisms fail before guest setup.
- Ruling: implement in this checkout on a research branch, preserving the supplied workspace and using small commits. No extra worktree is required by the user's request.
- Ruling: a missing guest image is an integration prerequisite, not permission to claim ordinary-distribution validation from the synthetic guest.
- Part 2: replaced deterministic clone identities with host-kernel random IDs/MACs and a private entropy payload. Guest credits and force-reseeds its CRNG before NIC hotplug. New host tests passed; a three-clone live run including parking and all-clone RAM/disk isolation passed. General userspace identity/key rotation remains an integration task.
- Part 3: added all-clone random divergence, matching all-live-clone memory snapshots, and `stats` with strict missing-data checks. An eight-clone 25%-of-32-MiB-buffer divergence run, parking, and all-clone RAM/disk/identity checks passed. The 32 MiB buffer is one eighth of configured guest RAM; full-guest divergence remains a distinct experiment.
- Part 4: KSM restore tests and a live SIGTERM test passed. The live test confirmed the supervisor reaped its QEMU child. Positive KSM experiments are blocked by host sysfs permissions; an attempted opt-in run failed with permission denied and `run` remained 0.
- Part 5: all six eight-clone divergence points completed successfully; raw JSON, a summary, and a matplotlib graph are saved. Process PSS grew from 316 MiB to 575 MiB while every clone independently dirtied its 32 MiB buffer; conventional process PSS was roughly 1.5 GiB. Full template cache residency is reported separately.
- Part 6: a checksum-verified Alpine 3.24.2 ext4 guest now boots BusyBox init, syslogd, crond, and getty, with an observed login prompt. Two restored clones passed RAM, filesystem, random-identity, process-kill, and parking checks. A fragmented-serial regression test caught and fixed a host prompt-reader bug. SSH and DHCP are absent in this profile.
