# Persistent clone lifecycle and crash reconciliation

`densevm clone start ID --template MANIFEST --state-dir DIR` creates a
per-clone registry record and qcow2 overlay, starts a separate QEMU process,
starts a separate QEMU process, restores the machine state while paused,
prepares guest entropy and identity, then attaches a new virtio NIC through
QMP and records the clone as `running`. The restored template must not contain
a NIC device; the template QEMU command must provide one `pcie-root-port` and a
restricted user-network backend. DenseVM checks the guest-visible MAC after
hotplug. See [`clone-network-identity.md`](clone-network-identity.md) for the
ordering and proof limits. `densevm clone stop ID --state-dir DIR` asks that
clone's QMP endpoint to quit and waits for it to close.

After QMP closes, stop also waits for the recorded QEMU PID to disappear from
`/proc` before marking the clone `stopped`. Closing QMP can precede process exit
briefly while QEMU finishes cleanup. If either signal does not arrive before the
timeout, the record remains `running` so callers can inspect or reconcile it.

Remove the state and private disk overlay for a stopped or reconciled failed
clone with:

```sh
densevm clone delete ID --state-dir DIR
```

Deletion refuses `starting`/`running` records, a still-live recorded PID, or a
reachable QMP endpoint. It also refuses artifact paths outside the clone's own
state directory. On success the per-clone record, overlay, sockets, and log
directory are removed; the shared template and backing disk are not touched.

A QEMU crash can leave a durable record saying `running`. Reconcile records
with:

```sh
densevm clone reconcile --state-dir DIR
```

The command returns the full registry after checking `running` and `starting`
records. Current records preserve each QEMU PID with its Linux boot ID and
`/proc/<pid>/stat` start-time ticks, so PID reuse does not count as the original
process. Old records without those fingerprints fall back to checking whether
the saved PID exists. A reachable QMP socket is treated as evidence that the VM
may still be alive, even if its PID is stale, so reconciliation leaves that
record unchanged. Other inspection errors stop reconciliation instead of
guessing.

Current `starting` records also contain the launcher PID, boot ID, and
start-time ticks. The QEMU PID fingerprint is written as soon as QEMU starts.
Reconcile marks a start `failed` only when the original launcher and recorded
QEMU process are both gone and the saved QMP endpoint is unreachable. If QMP is
reachable, it does not adopt the process or issue QMP commands; the record
remains `starting` for manual inspection. Legacy `starting` records without
launcher fingerprints remain unchanged because their launch state cannot be
distinguished safely.

The real-QEMU lifecycle check in
[`persistent-lifecycle-run1`](../results/runtime-clone/2026-10-26/persistent-lifecycle-run1/README.md)
exercises both a normal stop and a `SIGKILL` followed by reconciliation. The
normal stop returned only after its PID disappeared; the killed clone was
marked `failed`. This check verifies process lifecycle and registry behavior,
not guest-level isolation or memory savings.
