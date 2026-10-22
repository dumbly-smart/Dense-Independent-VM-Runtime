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
other init systems, SSH key rotation, DHCP identity, and arbitrary application
PRNG caches still require separate integration tests.
