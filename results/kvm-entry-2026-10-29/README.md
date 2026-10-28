# KVM guest-entry failure on 2026-10-29

A one-vCPU conventional baseline was run with the short output path below. It
failed before guest `READY`; the QEMU log reports `KVM: entry failed, hardware
error 0x0`.

```sh
cargo run -- benchmark baseline --out results/k1 --count 1 --accel kvm
```

The post-change two-vCPU probe also failed at its first conventional baseline
with the same message; its raw log is saved in
[`../vcpu-topology-2026-10-29/baseline-000.qemu.log`](../vcpu-topology-2026-10-29/baseline-000.qemu.log).
Together these runs show that current KVM guest execution fails for both
one- and two-vCPU starts on this host. They do not identify the underlying
host cause. `densevm doctor` can open `/dev/kvm`, which is weaker than proving
that KVM can execute a guest. No clone or template result is present here.

The first attempt used a longer output path and failed earlier because the
Unix-domain serial socket path exceeded the platform limit; that attempt is
excluded from the KVM execution conclusion.
