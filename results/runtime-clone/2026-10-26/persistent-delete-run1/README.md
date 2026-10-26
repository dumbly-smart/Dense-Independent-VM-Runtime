# Real-QEMU stopped-clone deletion

Using the saved QEMU 11.1.2 generation, this run started a persistent clone,
stopped it through QMP, then invoked `densevm clone delete`. The command
returned `{"deleted":"delete-check"}`. The clone state directory and qcow2
overlay were removed, the shared template disk hash was unchanged, and the
recorded QEMU PID was gone. The raw JSON contains all three CLI outputs.

This verifies cleanup after a normal stop. Refusal paths for live clones,
reachable QMP endpoints, and artifact paths outside the clone directory are
covered by the `tests/reconcile.rs` integration test.
