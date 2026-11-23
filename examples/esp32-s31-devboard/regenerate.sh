#!/bin/sh
# Rebuild the ESP32-S31 dev board from its replay and write every output.
# Pass --rebase after changing the shipped generators.
# Run from the repository root:  sh examples/esp32-s31-devboard/regenerate.sh
#
# 1. The replay (648 JSON-lines requests through `sparksmith stdio
#    --operator`, the same command engine the desktop and MCP use) opens the
#    module example, authors the other parts, draws the schematic, lays out
#    and routes the board and saves esp32-s31-devboard.sparksmith. Any refused
#    request fails.
# 2. `fab` writes the production files (byte-identical: --date none).
# 3. The schematic PDF and the native Solidcraft model are written beside it.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
export SPARKSMITH_LIBRARY_DIR="${SPARKSMITH_LIBRARY_DIR:-$root/library}"
dir=examples/esp32-s31-devboard
cli=${SPARKSMITH_CLI:-target/nix/ci/sparksmith}
[ -x "$cli" ] || cli=target/ci/sparksmith
[ -x "$cli" ] || { echo "build first: cargo build --locked --profile ci -p sparksmith-cli" >&2; exit 1; }

case "$#" in
    0) ;;
    1) [ "$1" = "--rebase" ] || { echo "usage: $0 [--rebase]" >&2; exit 1; }
       SPARKSMITH_CLI="$cli" python3 -B "$dir/authoring/rebase.py" ;;
    *) echo "usage: $0 [--rebase]" >&2; exit 1 ;;
esac

replies=$(mktemp)
# --operator: your own replay, not a model's (relative paths, ungated).
"$cli" stdio --operator < "$dir/esp32-s31-devboard.replay.jsonl" > "$replies"
if grep -q '"ok":false' "$replies"; then
    grep -n '"ok":false' "$replies" | head -5 >&2
    echo "the replay was refused; nothing else written" >&2
    rm -f "$replies"
    exit 1
fi
echo "replay: $(wc -l < "$replies") requests accepted"
rm -f "$replies"

rm -rf "$dir/fab" "$dir/esp32-s31-devboard-schematic.pdf" "$dir/esp32-s31-devboard.solidcraft"
"$cli" fab "$dir/esp32-s31-devboard.sparksmith" "$dir/fab" --date none > /dev/null
"$cli" export-schematic-pdf "$dir/esp32-s31-devboard.sparksmith" "$dir/esp32-s31-devboard-schematic.pdf" > /dev/null
if [ "${NATIVE:-0}" = 1 ]; then
    # About 10 s of CPU (see docs/e2e-esp32-devboard.md, gap 3).
    "$cli" export "$dir/esp32-s31-devboard.sparksmith" "$dir/esp32-s31-devboard.solidcraft" > /dev/null
fi
echo "wrote $dir/esp32-s31-devboard.sparksmith, $dir/fab/ and the schematic PDF"
