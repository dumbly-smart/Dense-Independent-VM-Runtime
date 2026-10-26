# Real-QEMU crash recovery and orphan deletion

This run started a persistent QEMU clone, sent `SIGKILL` to its recorded PID,
waited for the process to disappear, and ran `densevm clone reconcile`. The
registry changed from `running` to `failed`; `densevm clone delete` then removed
the failed clone's qcow2 overlay and record. The shared base disk hash remained
unchanged, and no QEMU process remained. Raw command output is in `result.json`.

This verifies cleanup after a QEMU process crash. It does not simulate a host
crash, power loss, or a partially written registry record.
