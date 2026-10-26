# Real-QEMU fingerprinted crash reconciliation

This run started a persistent clone, sent `SIGKILL` to its recorded QEMU PID,
and waited for that process to exit. The running registry record included the
QEMU PID, start-time ticks, and Linux boot ID. Reconciliation changed the
record to `failed`, then safe deletion removed only that clone's state and
overlay. The shared base disk hash was unchanged and no QEMU process remained.

This tests a QEMU process crash, not a host reboot or power loss. Raw CLI output
is in `result.json`.
