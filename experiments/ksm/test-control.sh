#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
source "$repo_root/experiments/ksm/control.sh"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
run_file=$tmp/run
printf '0\n' >"$run_file"

# Direct mode writes only the supplied control path and verifies the value.
write_ksm_run "$run_file" 1 direct
[[ $(<"$run_file") == 1 ]]

# Elevated mode invokes sudo only for tee; DenseVM is never run through sudo.
mkdir "$tmp/bin"
cat >"$tmp/bin/sudo" <<'MOCK_SUDO'
#!/usr/bin/env bash
[[ $1 == -n && $2 == tee ]] || exit 90
shift 2
exec tee "$@"
MOCK_SUDO
chmod +x "$tmp/bin/sudo"
PATH="$tmp/bin:$PATH" write_ksm_run "$run_file" 0 sudo
[[ $(<"$run_file") == 0 ]]

if PATH="$tmp/bin:$PATH" write_ksm_run "$run_file" 2 sudo; then
  echo 'invalid KSM run value unexpectedly accepted' >&2
  exit 1
fi

echo 'KSM control helper tests passed'
