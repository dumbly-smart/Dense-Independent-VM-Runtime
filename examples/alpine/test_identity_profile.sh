#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(CDPATH= cd -- "$script_dir/../.." && pwd)
rootfs=${1:?usage: test_identity_profile.sh ROOTFS_TAR ROOTFS_SHA APK_STATIC APK_SHA OUTPUT_DIR}
rootfs_sha=${2:?expected rootfs SHA256 required}
apk_static=${3:?apk-tools-static archive required}
apk_sha=${4:?expected apk-tools-static SHA256 required}
out=${5:?new output directory required}
builder_output=$(
    sh "$script_dir/prepare.sh" "$rootfs" "$rootfs_sha" "$out" \
        --identity-validation "$apk_static" "$apk_sha"
)

test -f "$builder_output/metadata.json"
jq -e '(.identity_validation == true) and (.ssh | startswith("OpenSSH")) and (.dhcp | startswith("BusyBox udhcpc"))' \
    "$builder_output/metadata.json" >/dev/null
grep -q '^P:openssh-server$' "$builder_output/root/lib/apk/db/installed"
grep -q '^P:openssh-keygen$' "$builder_output/root/lib/apk/db/installed"
grep -q '^::sysinit:/usr/bin/ssh-keygen -A$' "$builder_output/root/etc/inittab"
test -f "$builder_output/root/etc/densevm-identity-validation"
printf '%s\n' "$builder_output"
