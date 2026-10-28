# QMP vCPU topology and register smoke test

A paused two-vCPU QEMU 11.1.2 process under TCG returned `cpu-index` values
`[0, 1]` from `query-cpus-fast`. The HMP command `info registers -a` returned
separate `CPU#0` and `CPU#1` register blocks. The exact launch command and raw
monitor text are saved in `result.json`.

This verifies the locally installed QEMU interface and output format. TCG is
used only for this stopped-interface check; it does not validate the KVM
snapshot/restore pipeline or prove cross-VM architectural-state isolation.
