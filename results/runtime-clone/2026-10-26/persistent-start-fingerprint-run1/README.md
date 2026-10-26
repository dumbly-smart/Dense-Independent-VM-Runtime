# Real-QEMU persistent process fingerprints

This run started a persistent clone from the saved QEMU 11.1.2 template. Its
running record contained the QEMU PID, `/proc` start-time ticks, and Linux boot
ID; after startup completed, the launcher fingerprint had been cleared. Normal
QMP stop returned only after the QEMU PID disappeared, and the shared template
disk hash remained unchanged.

Raw start/stop output and the captured registry fields are in `result.json`.
