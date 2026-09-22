# Semantic status after the first probe

`YES` below means the named behavior was observed in the synthetic two-clone
experiment. `UNKNOWN` means the behavior remains untested for an ordinary
Linux distribution. The reference is QEMU/KVM 11.1.1 on Linux 7.2.6.

| Property | Conventional VM | DenseVM probe | Evidence or limit |
| --- | --- | --- | --- |
| Separate QEMU process | YES | YES | Two distinct PIDs; killing A left B responsive. |
| Guest RAM writes isolated | YES | YES | A wrote 32 MiB; B retained the original value. |
| Shared unchanged RAM | optional KSM | YES | `/proc/<pid>/smaps` showed about 22 MiB shared in each RAM-file mapping before divergence. |
| Disk mutations isolated | YES | YES | A and B wrote different bytes to their own overlays and read them back independently. |
| Shared disk backing | optional | YES | `qemu-img info --backing-chain` resolved both overlays to one qcow2 base. |
| Independent vCPU/register state | YES | UNKNOWN | Separate KVM/QEMU instances restore VMState, but no register-level adversarial test yet. |
| Independent virtual device state | YES | PARTIAL | Separate QMP, serial, block and hotplugged NIC observed; timer/interrupt behavior untested. |
| Unique MAC visible inside guest | YES | YES | Guest sysfs returned distinct MACs after NIC hotplug. Launch-time MAC changes alone failed because migration restored the template MAC. |
| Unique vmgenid in QEMU | optional | YES | QMP returned three distinct GUIDs for template, A, and B. Guest reaction to generation change is UNKNOWN. |
| Independent machine-id, SSH keys, hostname | YES when configured | UNKNOWN | Synthetic initramfs does not run those services; no ordinary distribution identity protocol yet. |
| Independent entropy/RNG state | expected | UNKNOWN | No guest entropy test or RNG reseed proof. |
| Independent DHCP identity/IP | expected | UNKNOWN | Network access is restricted; no DHCP client runs in the probe. |
| Independent wall clock/timers | YES | UNKNOWN | Time and timer divergence after restore have not been tested. |
| Kernel and KVM memory overhead included in result | applicable | UNKNOWN | Process PSS and template file cache residency are measured; kernel metadata is not isolated. |

The host's KSM `run` value was `0` in the reference experiment. This result is
template file COW, not KSM. Each guest still executes its own instructions on
KVM; no CPU work is deduplicated. The source RAM file and disk base are made
read-only after freezing and their backing relationships and hashes are
checked by the probe, but a privileged host actor can still change them.
