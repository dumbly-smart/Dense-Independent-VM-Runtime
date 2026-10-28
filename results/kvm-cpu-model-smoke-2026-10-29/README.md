# KVM CPU-model comparison on 2026-10-29

A one-vCPU conventional QEMU guest used `-cpu qemu64` with the same prepared
guest kernel and initramfs as the DenseVM baseline. It failed before serial
output with `KVM: entry failed, hardware error 0x0`. The launch arguments and
captured QEMU stderr are in `result.json`.

This rules out only the narrow hypothesis that this observed failure requires
`-cpu host`: the generic CPU model failed too. It does not identify the host
cause or test template restore.
