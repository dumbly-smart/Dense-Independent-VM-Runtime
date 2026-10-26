# Persistent clone lifecycle run 1

This run exercised persistent clone start, normal stop, and crash recovery
against a real saved QEMU template. The JSON file preserves every CLI command
and its raw output. Temporary clone state was removed after verifying that no
QEMU process remained.

## Environment

- QEMU: 11.1.2
- Linux: `7.2.11-arch1-1`
- Template generation: `sha256:caf6537b878472dce42d33b721c2ecd774b5dd996a8c7a2eb3bb0f28f03a9357`
- Guest RAM: 256 MiB
- KVM/template artifact validation: passed before the run

## Procedure

Using one temporary state directory, the run:

1. Started `lifecycle-stop` from the saved template.
2. Requested a normal QMP stop and checked that the recorded PID was gone
   before the command returned.
3. Started `lifecycle-crash` from the same template.
4. Sent `SIGKILL` to that QEMU process, waited for its PID to disappear, then
   ran `clone reconcile`.

Both clones reached `running` after start. The normal stop reached `stopped`
with the PID gone on return. Reconciliation marked the killed clone `failed`
and retained the normally stopped clone as `stopped`. Their generated MAC
addresses and machine IDs were distinct.

Reproduce with a validated local generation manifest:

```sh
STATE_DIR=$(mktemp -d)
densevm clone start stop-check --template /path/to/generation.json --state-dir "$STATE_DIR"
densevm clone stop stop-check --state-dir "$STATE_DIR"
densevm clone start crash-check --template /path/to/generation.json --state-dir "$STATE_DIR"
# Send SIGKILL to the qemu_pid reported by the preceding command.
densevm clone reconcile --state-dir "$STATE_DIR"
```

The `SIGKILL` step is intentionally manual in this reproduction snippet so the
operator can target only the reported QEMU PID. This experiment checks the
process and registry lifecycle. It does not measure RAM sharing or establish
guest-level isolation.
