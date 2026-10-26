# Persistent clone lifecycle and crash reconciliation

`densevm clone start ID --template MANIFEST --state-dir DIR` creates a
per-clone registry record and qcow2 overlay, starts a separate QEMU process,
restores the machine state, prepares guest identity, and records the clone as
`running`. `densevm clone stop ID --state-dir DIR` asks that clone's QMP
endpoint to quit and waits for it to close.

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

The command returns the full registry after checking each `running` record. It
changes a record to `failed` only when the recorded PID is absent from `/proc`
and the saved QMP socket is absent or refuses connections. A reachable QMP
socket is treated as evidence that the VM may still be alive, even if the
recorded PID is stale, so reconciliation leaves that record unchanged. Other
inspection errors also stop reconciliation instead of guessing.

`starting` records are deliberately left alone because records do not yet
contain a start time or boot ID that would let reconciliation distinguish a
crash from an in-progress launch. A reachable socket is checked only for
connectability; reconciliation does not adopt a new PID or issue QMP commands.
This favors avoiding a false failure report over repairing every ambiguous
orphan.

The real-QEMU lifecycle check in
[`persistent-lifecycle-run1`](../results/runtime-clone/2026-10-26/persistent-lifecycle-run1/README.md)
exercises both a normal stop and a `SIGKILL` followed by reconciliation. The
normal stop returned only after its PID disappeared; the killed clone was
marked `failed`. This check verifies process lifecycle and registry behavior,
not guest-level isolation or memory savings.
