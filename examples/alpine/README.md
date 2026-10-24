# Disk-backed Alpine guest

This profile boots Alpine's BusyBox init, syslogd, crond, a serial getty, and
the DenseVM agent on an ext4 root filesystem. Every clone gets its own qcow2
root overlay. Disk probes use `/var/lib/densevm/disk-probe.bin`; they never
overwrite raw filesystem blocks. No SSH server or DHCP client is installed.
The default locked root account is retained. A login prompt is available on
the separate `*.login.sock` serial endpoint inside the private output directory.

Download the official [Alpine 3.24.2 mini root filesystem](https://dl-cdn.alpinelinux.org/alpine/v3.24/releases/x86_64/)
and verify the [published checksum](https://dl-cdn.alpinelinux.org/alpine/v3.24/releases/x86_64/alpine-minirootfs-3.24.2-x86_64.tar.gz.sha256).
Preparation accepts a local archive and an explicit trusted SHA256. It does
not download packages, mount host filesystems, or require root.

`fakeroot` preserves archive UID/GID metadata while `mkfs.ext4 -d` builds the
image without host root privileges. The builder verifies that `/etc/passwd`
is owned by guest root; extracting as the host user without this step would
silently change guest filesystem permissions.

```sh
curl -fL -o /tmp/alpine-minirootfs-3.24.2-x86_64.tar.gz https://dl-cdn.alpinelinux.org/alpine/v3.24/releases/x86_64/alpine-minirootfs-3.24.2-x86_64.tar.gz
sh examples/alpine/prepare.sh /tmp/alpine-minirootfs-3.24.2-x86_64.tar.gz c5ca053cfe1d85c5b96dff8b9bc57045f7f184a30ffb6b65776409ca90388677 /tmp/densevm-alpine-guest
cargo run -- probe --guest /tmp/densevm-alpine-guest --count 2 --out /tmp/densevm-alpine-run
```

The first profile requires the current host kernel's ext4 support built in,
matching network modules, x86-64, rustc, cpio, fakeroot, mkfs.ext4, debugfs, qemu-img, jq, and
sha256sum. Guest hashes are checked before use. Prepared directories must be
kept intact for the experiment. This is a small supported distribution profile;
other init systems and arbitrary application PRNG caches still require separate
integration tests.

## Opt-in identity-validation profile

The optional `--identity-validation` profile adds OpenSSH from Alpine v3.24's
signed package repository. It uses the root filesystem's BusyBox `udhcpc` for a
bounded DHCP test. At boot, the template generates initial SSH host keys. Each
clone must reseed the guest kernel RNG before the identity protocol removes and
regenerates those keys. After clone identity preparation and NIC attachment,
`udhcpc` sends DHCP option 61 derived from the clone's machine-id; the guest
captures the emitted Ethernet packet with an `AF_PACKET` socket and checks the
option bytes before reporting success. The QEMU user-mode DHCP server must
offer a lease for the run to pass.

The mode accepts a locally downloaded, SHA-256-pinned `apk-tools-static`
archive. Package scripts are disabled during alternate-root installation, so
the guest does not run untrusted package hooks on the host. OpenSSH is pinned
to `10.3_p1-r1`; the fixture metadata records the `apk-tools-static` archive
hash and installed package database hash. The downloaded `apk.static` helper
is retained in the prepared output directory as tool provenance.

```sh
curl -fL -o /tmp/alpine-minirootfs-3.24.2-x86_64.tar.gz \
  https://dl-cdn.alpinelinux.org/alpine/v3.24/releases/x86_64/alpine-minirootfs-3.24.2-x86_64.tar.gz
curl -fL -o /tmp/apk-tools-static-3.0.8-r0.apk \
  https://dl-cdn.alpinelinux.org/alpine/v3.24/main/x86_64/apk-tools-static-3.0.8-r0.apk
sh examples/alpine/prepare.sh \
  /tmp/alpine-minirootfs-3.24.2-x86_64.tar.gz \
  c5ca053cfe1d85c5b96dff8b9bc57045f7f184a30ffb6b65776409ca90388677 \
  /tmp/densevm-alpine-identity \
  --identity-validation \
  /tmp/apk-tools-static-3.0.8-r0.apk \
  c8e2c88c13ba12a12269b79a3543e1190ff8c0ab0beb32b58cadfd5881c619e3
cargo run -- probe --guest /tmp/densevm-alpine-identity --count 2 \
  --skip-guest-oom --out /tmp/densevm-identity-run
```

The two-clone identity run verifies unique guest machine-ids, three unique SSH
host-key fingerprints per guest, distinct DHCP option 61 values observed in
the actual client packets, and successful lease acquisition. `--skip-guest-oom`
skips only the separate guest OOM workload and is reflected as `status: skipped`
in the result. This fixture validates its BusyBox client; it does not establish
that every distribution's DHCP client derives or rotates its identity the same
way. Other DHCP implementations remain separate questions. In the recorded
run, both clones also logged one Linux kernel
`crng reseeded due to virtual machine fork` event relative to the template
baseline before DenseVM injected its per-clone entropy payload. This verifies
the VMGenID response for the tested Alpine kernel; it does not prove that
userspace PRNGs or applications refreshed cached random state.

The profile builder integration check is:

```sh
sh examples/alpine/test_identity_profile.sh ROOTFS_TAR ROOTFS_SHA256 \
  APK_STATIC_APK APK_STATIC_SHA256 OUTPUT_DIR
```
