#!/usr/bin/env bash
# Exercise capture setup without launching a desktop or recording a screen.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
source "$here/state.sh"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir "$scratch/first"
capture_state "$scratch/runs" "$scratch/first"
first="$(cat "$scratch/first/state.path")"
echo pending > "$SPARKSMITH_RECOVERY_DIR/snapshot"
# Calibration has no output directory. Its metadata lives with the run.
(capture_state "$scratch/runs"; cat "$state/state.path" > "$scratch/second")
second="$(cat "$scratch/second")"
[[ "$first" != "$second" && "$(cat "$first/recovery/snapshot")" = pending ]]
for run in "$first" "$second"; do
  for dir in home recovery config cache; do [[ -d "$run/$dir" ]]; done
done
