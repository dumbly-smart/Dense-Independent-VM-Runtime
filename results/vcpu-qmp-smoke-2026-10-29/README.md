# QMP vCPU topology query smoke test

A paused two-vCPU QEMU 11.1.2 process under TCG returned `cpu-index` values
`[0, 1]` from `query-cpus-fast`. The exact command and raw structured reply are
in `result.json`. This confirms the installed QEMU command and response shape;
it does not establish KVM template-restore behavior.
