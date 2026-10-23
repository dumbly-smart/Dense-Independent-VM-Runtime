# Attacker/client plus five independent targets

Prepare a fresh Alpine guest using `examples/alpine/README.md`, then run:

```sh
cargo run -- probe --guest /tmp/densevm-alpine-guest --range --count 6 --out /tmp/densevm-range
```

The last clone is the client/attacker role, and clones 0–4 are targets. Each
has a private ext4 root overlay, RAM writes, kernel RNG reseed, identity, MAC,
static IPv4 address, QEMU process, and QMP/serial endpoints. The shared Ethernet
segment uses QEMU's socket multicast backend bound to host loopback. No tap,
bridge, host forwarding rule, or root permission is needed. The network is
intentionally shared among these guests; other local host processes can join
the multicast group. It is not isolation against hostile host users.

After identity preparation, every guest receives an address in 10.77.0.0/24
and an in-guest UDP identity responder. The client contacts all five targets.
It also sends a 64-packet, 1 KiB UDP burst to T1 and records replies, loss,
elapsed time, request rate, payload goodput, and target identity. This is a short
guest-to-guest diagnostic over the local QEMU multicast backend, not host NIC
line rate.
The experiment writes private RAM/filesystem state, starts a process only in
T1, attempts its reboot, and kills T1's QEMU process. It verifies sibling RAM,
filesystem data, identity, liveness, and continued UDP reachability of T2.
Results are saved in `result.json`; all QEMU processes are reaped on exit.

This example demonstrates network/device independence and destructive target
isolation. The UDP responder is a diagnostic service, not an installed CVE or
a claim of a successful exploitation exercise. SSH, DHCP, heavy-traffic
benchmarks, kernel-crash/OOM tests, and 20-target scale need separate runs.

The template and clones must have matching network backend types: even an
unconnected user/slirp backend contributes migration state. Changing a source
user backend into a destination socket backend caused a real restore failure
in development; the runtime now constructs matching template/clone backends.
