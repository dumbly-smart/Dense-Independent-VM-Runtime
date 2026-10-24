#!/usr/bin/env bash
set -uo pipefail

usage() {
  cat <<'USAGE'
Usage: experiments/cgroup/run-probe.sh --out DIR [probe options]

Run a DenseVM probe in a dedicated delegated cgroup-v2 memory scope. The
wrapper creates/removes only its own child cgroup; it does not enable
controllers or set memory limits. Probe JSON includes cgroup memory.current and
memory.stat at baseline-ready and clone divergence snapshots.
USAGE
}

out=
probe_args=()
while (($#)); do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --out)
      (($# >= 2)) || { usage >&2; exit 2; }
      out=$2
      probe_args+=(--out "$2")
      shift 2
      ;;
    *) probe_args+=("$1"); shift ;;
  esac
done
if [[ -z "$out" ]]; then
  echo '--out is required' >&2
  exit 2
fi
if [[ -e "$out" ]]; then
  echo "output path already exists: $out" >&2
  exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
densevm_bin=${DENSEVM_BIN:-"$repo_root/target/debug/densevm"}
if [[ ! -x "$densevm_bin" ]]; then
  echo "DenseVM executable is missing or not executable: $densevm_bin" >&2
  exit 2
fi

relative=$(awk -F: '$1 == "0" { print $3 }' /proc/self/cgroup)
if [[ -z "$relative" ]]; then
  echo 'cgroup v2 membership was not found in /proc/self/cgroup' >&2
  exit 2
fi
source_cgroup="/sys/fs/cgroup$relative"
if [[ ! -d "$source_cgroup" ]]; then
  echo "cgroup v2 path is not mounted at the expected location: $source_cgroup" >&2
  exit 2
fi

candidate=$source_cgroup
accounting_parent=
while [[ "$candidate" != /sys/fs/cgroup && "$candidate" != / ]]; do
  candidate=$(dirname "$candidate")
  if [[ -w "$candidate" && -r "$candidate/cgroup.subtree_control" ]] \
    && grep -qw memory "$candidate/cgroup.subtree_control"; then
    accounting_parent=$candidate
    break
  fi
done
if [[ -z "$accounting_parent" ]]; then
  echo 'no writable ancestor has the cgroup-v2 memory controller enabled; refusing to change controller state' >&2
  exit 3
fi

cgroup="$accounting_parent/densevm-probe-$$-$(date +%s%N)"
if ! mkdir "$cgroup"; then
  echo "cannot create dedicated memory-accounted cgroup under $accounting_parent" >&2
  exit 3
fi
# shellcheck disable=SC2329 # Invoked by the EXIT trap.
cleanup() {
  local status=$?
  if ! rmdir "$cgroup" 2>/dev/null; then
    echo "densevm: could not remove cgroup $cgroup; check for surviving probe processes" >&2
  fi
  return "$status"
}
trap cleanup EXIT
if [[ ! -r "$cgroup/memory.current" || ! -r "$cgroup/memory.stat" ]]; then
  echo 'new child cgroup has no memory accounting files; refusing to launch DenseVM' >&2
  exit 3
fi

DENSEVM_CGROUP_PATH="$cgroup" bash -c '
  printf "%s\n" "$BASHPID" > "$DENSEVM_CGROUP_PATH/cgroup.procs" || exit 125
  exec "$@"
' densevm-cgroup-exec "$densevm_bin" probe "${probe_args[@]}"
exit $?
