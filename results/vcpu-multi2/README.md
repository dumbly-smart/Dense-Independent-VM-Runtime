# Two-vCPU restore samples

`result.json` and `../vcpu-multi2-run2/result.json` contain the machine-readable
evidence for two repeated two-vCPU KVM template/restore runs. Every baseline,
template, and clone QEMU command used `-smp 2`. Both runs passed the recorded
RAM and disk isolation, backing-chain, identity uniqueness, vCPU0 snapshot, and
sibling-survival checks.

The 256 MiB template `memory.img` and 64 MiB virtual qcow2 disk files are kept
in the local experiment directories but are excluded from Git: GitHub rejects
files over 100 MiB and warns at 50 MiB. The JSON retains the measurements,
commands, host metadata, and guest-visible checks. Recreate the experiments on
a host with QEMU/KVM and adequate memory using:

```sh
cargo run -- probe --out /path/to/vcpu-multi2 --count 2 \
  --vcpus 2 --vcpu-register-state --skip-guest-oom
cargo run -- probe --out /path/to/vcpu-multi2-run2 --count 2 \
  --vcpus 2 --vcpu-register-state --skip-guest-oom
```

HMP register output contains `CPU#0` only. These runs demonstrate that a
two-vCPU template restored and passed the listed isolation probes; they do not
directly compare vCPU1 architectural registers between guests.
