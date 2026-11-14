#!/usr/bin/env bash

# Write and verify only the KSM run control. In sudo mode, privilege is
# limited to tee; callers must not run DenseVM itself through sudo.
write_ksm_run() {
  local path=$1 value=$2 mode=$3
  [[ "$value" == 0 || "$value" == 1 ]] || {
    echo "refusing unsupported KSM run value: $value" >&2
    return 2
  }

  case "$mode" in
    direct) printf '%s\n' "$value" >"$path" ;;
    sudo) printf '%s\n' "$value" | sudo -n tee "$path" >/dev/null ;;
    *) echo "unknown KSM write mode: $mode" >&2; return 2 ;;
  esac || return $?

  local observed
  observed=$(cat "$path") || return 1
  if [[ "$observed" != "$value" ]]; then
    echo "KSM run verification failed: expected $value, found $observed" >&2
    return 1
  fi
}
