# Interrupted KVM vCPU topology run

The 2026-10-29 rerun used:

```sh
cargo run -- probe --out results/vcpu-topology-2026-10-29 --count 2 \
  --vcpus 2 --vcpu-register-state --skip-guest-oom
```

It failed while booting the first conventional baseline VM, before template
creation or clone restore. `baseline-000.qemu.log` reports `KVM: entry failed,
hardware error 0x0`; therefore this run provides no evidence about restored
vCPU topology. QEMU has `query-cpus-fast` support confirmed separately by the
TCG-only smoke result in `../vcpu-qmp-smoke-2026-10-29/`. `densevm doctor`
reported `/dev/kvm` accessible, but that check only opens the device and does
not prove guest execution works. The previously successful KVM runs remain
under `../vcpu-multi2/` and `../vcpu-multi2-run2/`.
