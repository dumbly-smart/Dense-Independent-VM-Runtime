#!/bin/sh
# Build an ext4-backed Alpine KVM test guest without mounting or chrooting.
set -eu
if [ -z "${FAKEROOTKEY:-}" ]; then
    command -v fakeroot >/dev/null
    exec fakeroot -- sh "$0" "$@"
fi
archive=$(realpath "${1:?usage: prepare.sh ROOTFS_TAR SHA256 NEW_OUTPUT_DIR [--identity-validation APK_STATIC_APK APK_SHA256]}")
checksum=${2:?expected SHA256 required}
out=${3:?new output directory required}
mode=${4:-minimal}
case "$checksum" in *[!0-9a-fA-F]*|'') printf '%s\n' 'invalid SHA256' >&2; exit 2;; esac
[ "${#checksum}" -eq 64 ] || exit 2
printf '%s  %s\n' "$checksum" "$archive" | sha256sum --check --status
identity_validation=false
apk_archive_sha256=
if [ "$mode" = "--identity-validation" ]; then
    [ "$#" -eq 6 ] || { printf '%s\n' 'identity profile requires APK_STATIC_APK and APK_SHA256' >&2; exit 2; }
    apk_archive=$(realpath "$5")
    apk_archive_sha256=$6
    case "$apk_archive_sha256" in *[!0-9a-fA-F]*|'') printf '%s\n' 'invalid apk-tools-static SHA256' >&2; exit 2;; esac
    [ "${#apk_archive_sha256}" -eq 64 ] || exit 2
    printf '%s  %s\n' "$apk_archive_sha256" "$apk_archive" | sha256sum --check --status
    identity_validation=true
elif [ "$mode" != "minimal" ] || [ "$#" -ne 3 ]; then
    printf '%s\n' 'usage: prepare.sh ROOTFS_TAR SHA256 NEW_OUTPUT_DIR [--identity-validation APK_STATIC_APK APK_SHA256]' >&2
    exit 2
fi
script=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(CDPATH= cd -- "$script/../.." && pwd)
# This first profile deliberately requires built-in ext4 support.
[ "$(modinfo -F filename ext4)" = '(builtin)' ] || { printf '%s\n' 'host kernel must have ext4 built in' >&2; exit 1; }
mkdir -m 700 "$out"
out=$(CDPATH= cd -- "$out" && pwd)
root="$out/root"
boot="$out/bootroot"
mkdir "$root"
tar --extract --gzip --file "$archive" --directory "$root" --same-owner
test -f "$root/etc/alpine-release"
mkdir -p "$root/modules" "$root/var/lib/densevm" "$root/proc" "$root/sys" "$root/dev" "$root/etc/crontabs"
if [ "$identity_validation" = true ]; then
    tar --extract --gzip --to-stdout --file "$apk_archive" sbin/apk.static > "$out/apk.static"
    chmod 755 "$out/apk.static"
    "$out/apk.static" --root "$root" --initdb --no-scripts --usermode add openssh=10.3_p1-r1 >&2
    loader="$root/lib/ld-musl-x86_64.so.1"
    busybox_list=$(LD_LIBRARY_PATH="$root/lib:$root/usr/lib" "$loader" "$root/bin/busybox" --list)
    printf '%s\n' "$busybox_list" | grep -qx udhcpc
    test -x "$root/usr/bin/ssh-keygen"
    test -x "$root/usr/sbin/sshd"
    touch "$root/etc/densevm-identity-validation"
    package_db_sha256=$(sha256sum "$root/lib/apk/db/installed")
    package_db_sha256=${package_db_sha256%% *}
else
    package_db_sha256=
fi
for module in failover net_failover virtio_net; do
    cp "$(modinfo -F filename "$module")" "$root/modules/$module.ko.zst"
done
rustc --edition=2024 -C target-feature=+crt-static -C panic=abort -O "$repo/experiments/cow-memory/guest-init.rs" -o "$root/sbin/densevm-agent"
if [ "$identity_validation" = true ]; then
    { printf '%s\n' '::sysinit:/usr/bin/ssh-keygen -A'; cat "$script/inittab"; } > "$root/etc/inittab"
else
    cp "$script/inittab" "$root/etc/inittab"
fi
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
debugfs -R 'stat /etc/passwd' "$out/root.raw" 2>/dev/null | grep -Eq 'User: +0 +Group: +0'
qemu-img convert -f raw -O qcow2 "$out/root.raw" "$out/disk.qcow2"
kernel_hash=$(sha256sum "$out/kernel")
initramfs_hash=$(sha256sum "$out/initramfs.cpio")
disk_hash=$(sha256sum "$out/disk.qcow2")
agent_hash=$(sha256sum "$root/sbin/densevm-agent")
jq -n --arg version "$(cat "$root/etc/alpine-release")" --arg source "$archive" --arg archive_sha256 "$checksum" --arg kernel_sha256 "${kernel_hash%% *}" --arg initramfs_sha256 "${initramfs_hash%% *}" --arg disk_sha256 "${disk_hash%% *}" --arg agent_sha256 "${agent_hash%% *}" --arg apk_static_sha256 "$apk_archive_sha256" --arg package_db_sha256 "$package_db_sha256" --argjson identity_validation "$identity_validation" '{kind:"alpine-ext4-v1",version:$version,source:$source,archive_sha256:$archive_sha256,kernel_sha256:$kernel_sha256,initramfs_sha256:$initramfs_sha256,disk_sha256:$disk_sha256,agent_sha256:$agent_sha256,identity_validation:$identity_validation,apk_static_sha256:$apk_static_sha256,package_db_sha256:$package_db_sha256,identity_validation_packages:(if $identity_validation then ["openssh=10.3_p1-r1","BusyBox udhcpc from the locked Alpine rootfs"] else [] end),services:(if $identity_validation then ["busybox-init","syslogd","crond","serial-getty","densevm-agent","ssh-keygen host-key generation"] else ["busybox-init","syslogd","crond","serial-getty","densevm-agent"] end),ssh:(if $identity_validation then "OpenSSH installed; initial host keys generated at boot and rotated after clone reseeding" else "not installed; no SSH host keys" end),dhcp:(if $identity_validation then "BusyBox udhcpc; client identifier is captured from the emitted DHCP request" else "not installed or exercised" end),network:"restricted user networking; no shared target LAN"}' > "$out/metadata.json"
chmod 444 "$out/kernel" "$out/initramfs.cpio" "$out/disk.qcow2" "$out/metadata.json"
printf '%s\n' "$out"
