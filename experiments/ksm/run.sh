#!/usr/bin/env bash
set -uo pipefail

usage() {
  cat <<'USAGE'
Usage: experiments/ksm/run.sh --out DIR [--count N] [--guest DIR]
       [--divergence-percent 0..100] [--include-guest-oom] [--check-only]

Runs matched template-COW and template-COW-plus-KSM probes. The output path
must be new. KSM must initially be disabled and writable by this process. The
script never invokes sudo and never writes KSM sysfs itself.
USAGE
}

out=
count=2
divergence_percent=0
guest=
skip_guest_oom=1
check_only=0
while (($#)); do
  case "$1" in
    --out) (($# >= 2)) || { usage >&2; exit 2; }; out=$2; shift 2 ;;
    --count) (($# >= 2)) || { usage >&2; exit 2; }; count=$2; shift 2 ;;
    --guest) (($# >= 2)) || { usage >&2; exit 2; }; guest=$2; shift 2 ;;
    --divergence-percent) (($# >= 2)) || { usage >&2; exit 2; }; divergence_percent=$2; shift 2 ;;
    --include-guest-oom) skip_guest_oom=0; shift ;;
    --check-only) check_only=1; shift ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'unknown option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ ! "$count" =~ ^[0-9]+$ ]]; then
  echo '--count must be an integer from 2 through 32' >&2; exit 2
fi
count_number=$((10#$count))
if ((count_number < 2 || count_number > 32)); then
  echo '--count must be an integer from 2 through 32' >&2; exit 2
fi
if [[ ! "$divergence_percent" =~ ^[0-9]+$ ]]; then
  echo '--divergence-percent must be an integer from 0 through 100' >&2; exit 2
fi
divergence_number=$((10#$divergence_percent))
if ((divergence_number > 100)); then
  echo '--divergence-percent must be an integer from 0 through 100' >&2; exit 2
fi
if ((check_only == 0)) && [[ -z "$out" ]]; then
  echo '--out is required unless --check-only is used' >&2; exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
densevm_bin=${DENSEVM_BIN:-"$repo_root/target/debug/densevm"}
ksm_run=/sys/kernel/mm/ksm/run
initial_run=
write_access=false
if [[ -r "$ksm_run" ]]; then initial_run=$(<"$ksm_run"); fi
if [[ -w "$ksm_run" ]]; then write_access=true; fi
status=ready
reason=
if [[ ! -r "$ksm_run" ]]; then
  status=blocked; reason='KSM run control is unavailable'
elif [[ "$initial_run" != 0 ]]; then
  status=blocked; reason="KSM must initially be disabled (run=$initial_run) for a controlled comparison"
elif [[ "$write_access" != true ]]; then
  status=blocked; reason='current user cannot enable host-global KSM; administrator access is required'
elif [[ ! -x "$densevm_bin" ]]; then
  status=blocked; reason="DenseVM executable is missing or not executable: $densevm_bin"
fi

if ((check_only)); then
  KSM_PREFLIGHT_STATUS=$status KSM_PREFLIGHT_REASON=$reason \
    KSM_PREFLIGHT_RUN=${initial_run:-null} KSM_PREFLIGHT_WRITABLE=$write_access \
    KSM_PREFLIGHT_BIN=$densevm_bin KSM_PREFLIGHT_COUNT=$count \
    KSM_PREFLIGHT_DIVERGENCE=$divergence_percent python3 - <<'PY'
import json, os
print(json.dumps({
    "status": os.environ["KSM_PREFLIGHT_STATUS"],
    "reason": os.environ["KSM_PREFLIGHT_REASON"] or None,
    "ksm_run_before": None if os.environ["KSM_PREFLIGHT_RUN"] == "null" else int(os.environ["KSM_PREFLIGHT_RUN"]),
    "ksm_run_writable": os.environ["KSM_PREFLIGHT_WRITABLE"] == "true",
    "densevm_binary": os.environ["KSM_PREFLIGHT_BIN"],
    "clone_count": int(os.environ["KSM_PREFLIGHT_COUNT"]),
    "divergence_percent": int(os.environ["KSM_PREFLIGHT_DIVERGENCE"]),
}, indent=2))
PY
  exit 0
fi

if [[ -e "$out" ]]; then
  echo "output path already exists: $out" >&2
  exit 2
fi
mkdir -p "$out" || exit 1
KSM_PREFLIGHT_STATUS=$status KSM_PREFLIGHT_REASON=$reason \
  KSM_PREFLIGHT_RUN=${initial_run:-null} KSM_PREFLIGHT_WRITABLE=$write_access \
  KSM_PREFLIGHT_BIN=$densevm_bin KSM_PREFLIGHT_COUNT=$count \
  KSM_PREFLIGHT_DIVERGENCE=$divergence_percent KSM_PREFLIGHT_GUEST=$guest \
  KSM_PREFLIGHT_SKIP_OOM=$skip_guest_oom python3 - "$out/preflight.json" <<'PY'
import json, os, sys
print(json.dumps({
    "status": os.environ["KSM_PREFLIGHT_STATUS"],
    "reason": os.environ["KSM_PREFLIGHT_REASON"] or None,
    "ksm_run_before": None if os.environ["KSM_PREFLIGHT_RUN"] == "null" else int(os.environ["KSM_PREFLIGHT_RUN"]),
    "ksm_run_writable": os.environ["KSM_PREFLIGHT_WRITABLE"] == "true",
    "densevm_binary": os.environ["KSM_PREFLIGHT_BIN"],
    "clone_count": int(os.environ["KSM_PREFLIGHT_COUNT"]),
    "divergence_percent": int(os.environ["KSM_PREFLIGHT_DIVERGENCE"]),
    "guest": os.environ["KSM_PREFLIGHT_GUEST"] or None,
    "guest_oom_skipped": os.environ["KSM_PREFLIGHT_SKIP_OOM"] == "1",
}, indent=2), file=open(sys.argv[1], "w"))
PY

if [[ "$status" != ready ]]; then
  echo "KSM experiment blocked: $reason" >&2
  exit 3
fi

common_args=(--count "$count" --divergence-percent "$divergence_percent")
[[ -z "$guest" ]] || common_args+=(--guest "$guest")
((skip_guest_oom == 0)) || common_args+=(--skip-guest-oom)

write_run_status() {
  local stage=$1 exit_code=$2 current=$3
  KSM_STAGE=$stage KSM_EXIT_CODE=$exit_code KSM_CURRENT_RUN=$current \
    KSM_INITIAL_RUN=$initial_run python3 - "$out/run-status.json" <<'PY'
import json, os, sys
print(json.dumps({
    "status": "failed",
    "stage": os.environ["KSM_STAGE"],
    "exit_code": int(os.environ["KSM_EXIT_CODE"]),
    "ksm_run_before": int(os.environ["KSM_INITIAL_RUN"]),
    "ksm_run_after_failure": int(os.environ["KSM_CURRENT_RUN"]) if os.environ["KSM_CURRENT_RUN"].isdigit() else None,
    "restored_to_initial": os.environ["KSM_CURRENT_RUN"] == os.environ["KSM_INITIAL_RUN"],
}, indent=2), file=open(sys.argv[1], "w"))
PY
}

record_unexpected_exit() {
  local exit_code=$?
  local current_run
  current_run=$(cat "$ksm_run" 2>/dev/null || true)
  if [[ ! -e "$out/run-status.json" && $exit_code -ne 0 ]]; then
    write_run_status interrupted-or-unexpected-exit "$exit_code" "$current_run"
  fi
  if [[ -n "$current_run" && "$current_run" != "$initial_run" ]]; then
    echo "densevm: KSM run value is $current_run after experiment exit; original value was $initial_run" >&2
  fi
}
trap record_unexpected_exit EXIT

if "$densevm_bin" probe --out "$out/template-cow" "${common_args[@]}"; then
  :
else
  result=$?
  write_run_status control "$result" "$(cat "$ksm_run" 2>/dev/null || true)"
  exit "$result"
fi
after_control=$(cat "$ksm_run" 2>/dev/null || true)
if [[ "$after_control" != "$initial_run" ]]; then
  write_run_status control-state-changed 1 "$after_control"
  echo 'KSM run value changed during the control run; refusing the KSM comparison' >&2
  exit 1
fi

if "$densevm_bin" probe --out "$out/template-cow-plus-ksm" "${common_args[@]}" --ksm; then
  :
else
  result=$?
  write_run_status ksm "$result" "$(cat "$ksm_run" 2>/dev/null || true)"
  exit "$result"
fi
after_ksm=$(cat "$ksm_run" 2>/dev/null || true)
if [[ "$after_ksm" != "$initial_run" ]]; then
  write_run_status ksm-state-restore 1 "$after_ksm"
  echo 'KSM did not return to its initial run value; inspect host state immediately' >&2
  exit 1
fi

python3 "$repo_root/experiments/ksm/summarize.py" \
  "$out/template-cow/result.json" \
  "$out/template-cow-plus-ksm/result.json" \
  "$out/comparison.json" || exit $?
KSM_INITIAL_RUN=$initial_run KSM_CONTROL_RUN=$after_control KSM_AFTER_RUN=$after_ksm \
  KSM_COUNT=$count KSM_DIVERGENCE=$divergence_percent KSM_GUEST=$guest \
  KSM_SKIP_OOM=$skip_guest_oom python3 - "$out/run-status.json" <<'PY'
import json, os, sys
print(json.dumps({
    "status": "complete",
    "ksm_run_before": int(os.environ["KSM_INITIAL_RUN"]),
    "ksm_run_after_control": int(os.environ["KSM_CONTROL_RUN"]),
    "ksm_run_after_ksm": int(os.environ["KSM_AFTER_RUN"]),
    "restored_to_initial": os.environ["KSM_AFTER_RUN"] == os.environ["KSM_INITIAL_RUN"],
    "clone_count": int(os.environ["KSM_COUNT"]),
    "divergence_percent": int(os.environ["KSM_DIVERGENCE"]),
    "guest": os.environ["KSM_GUEST"] or None,
    "guest_oom_skipped": os.environ["KSM_SKIP_OOM"] == "1",
}, indent=2), file=open(sys.argv[1], "w"))
PY
