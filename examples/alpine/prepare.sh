#!/bin/sh
# Build an ext4-backed Alpine KVM test guest without mounting or chrooting.
set -eu
archive=$(realpath "${1:?usage: prepare.sh ROOTFS_TAR SHA256 NEW_OUTPUT_DIR}")
checksum=${2:?expected SHA256 required}
out=${3:?new output directory required}
case "$checksum" in *[!0-9a-fA-F]*|'') printf '%s\n' 'invalid SHA256' >&2; exit 2;; esac
[ "${#checksum}" -eq 64 ] || exit 2
printf '%s  %s\n' "$checksum" "$archive" | sha256sum --check --status
script=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(CDPATH= cd -- "$script/../.." && pwd)
# This first profile deliberately requires built-in ext4 support.
[ "$(modinfo -F filename ext4)" = '(builtin)' ] || { printf '%s\n' 'host kernel must have ext4 built in' >&2; exit 1; }
mkdir -m 700 "$out"
out=$(CDPATH= cd -- "$out" && pwd)
root="$out/root"
boot="$out/bootroot"
mkdir "$root"
tar --extract --gzip --file "$archive" --directory "$root" --no-same-owner
test -f "$root/etc/alpine-release"
mkdir -p "$root/modules" "$root/var/lib/densevm" "$root/proc" "$root/sys" "$root/dev" "$root/etc/crontabs"
for module in failover net_failover virtio_net; do
    cp "$(modinfo -F filename "$module")" "$root/modules/$module.ko.zst"
done
rustc --edition=2024 -C target-feature=+crt-static -C panic=abort -O "$repo/experiments/cow-memory/guest-init.rs" -o "$root/sbin/densevm-agent"
cp "$script/inittab" "$root/etc/inittab"
truncate -s 4096 "$root/var/lib/densevm/disk-probe.bin"
mkdir -p "$boot/bin" "$boot/lib" "$boot/dev" "$boot/newroot"
cp "$root/bin/busybox" "$boot/bin/busybox"
cp -L "$root/lib/ld-musl-x86_64.so.1" "$boot/lib/ld-musl-x86_64.so.1"
cp "$script/init" "$boot/init"
chmod 755 "$boot/init"
(cd "$boot" && find . -print0 | cpio --null -o -H newc --quiet) > "$out/initramfs.cpio"
cp /boot/vmlinuz-linux "$out/kernel"
truncate -s 256M "$out/root.raw"
mkfs.ext4 -q -F -m 0 -d "$root" "$out/root.raw"
qemu-img convert -f raw -O qcow2 "$out/root.raw" "$out/disk.qcow2"
kernel_hash=$(sha256sum "$out/kernel")
initramfs_hash=$(sha256sum "$out/initramfs.cpio")
disk_hash=$(sha256sum "$out/disk.qcow2")
agent_hash=$(sha256sum "$root/sbin/densevm-agent")
jq -n --arg version "$(cat "$root/etc/alpine-release")" --arg source "$archive" --arg archive_sha256 "$checksum" --arg kernel_sha256 "${kernel_hash%% *}" --arg initramfs_sha256 "${initramfs_hash%% *}" --arg disk_sha256 "${disk_hash%% *}" --arg agent_sha256 "${agent_hash%% *}" '{kind:"alpine-ext4-v1",version:$version,source:$source,archive_sha256:$archive_sha256,kernel_sha256:$kernel_sha256,initramfs_sha256:$initramfs_sha256,disk_sha256:$disk_sha256,agent_sha256:$agent_sha256,services:["busybox-init","syslogd","crond","serial-getty","densevm-agent"],ssh:"not installed; no SSH host keys",network:"restricted user networking; no shared target LAN"}' > "$out/metadata.json"
chmod 444 "$out/kernel" "$out/initramfs.cpio" "$out/disk.qcow2" "$out/metadata.json"
printf '%s\n' "$out"
