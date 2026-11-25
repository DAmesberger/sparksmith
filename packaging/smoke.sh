#!/bin/sh
# Tries a package as a person gets it, outside the repository:
#   sh packaging/smoke.sh ARCHIVE WORK_DIR
# It unpacks ARCHIVE into the new folder WORK_DIR, asks both programs for
# their version, writes the production files of the ESP32 example with the
# command line and compares them with the committed ones, then starts the app
# on that example and waits until the app has noted it as opened. Linux needs
# a display (xvfb-run). Run from the repository root.
set -eu
archive=$1
work=$2
example=examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith
commit=$(git rev-parse --short=10 HEAD)
mkdir "$work"
work=$(CDPATH= cd -- "$work" && pwd -P)

case "$archive" in
    *.tar.xz) tar -xJf "$archive" -C "$work" ;;
    *) if command -v ditto >/dev/null 2>&1; then ditto -x -k "$archive" "$work"
       else unzip -q "$archive" -d "$work"; fi ;;
esac
package=$(echo "$work"/sparksmith-*)
case "$(uname -s)" in
    Linux) programs=$package/bin; x= ;;
    Darwin) programs=$package/Sparksmith.app/Contents/MacOS; x=
        codesign --verify --strict "$package/Sparksmith.app" ;;
    *) programs=$package; x=.exe ;;
esac

# Nothing of the build's machine helps: the package's own library is found.
unset SPARKSMITH_LIBRARY_DIR LD_LIBRARY_PATH
for program in sparksmith sparksmith-desktop; do
    version=$("$programs/$program$x" --version)
    echo "$program: $version"
    case "$version" in *"($commit)"*) ;; *) echo "not the commit $commit" >&2; exit 1 ;; esac
done

cp "$example" "$work/esp32-s31-devboard.sparksmith"
cd "$work"
"$programs/sparksmith$x" fab esp32-s31-devboard.sparksmith fab --date none > fab.json
cd "$OLDPWD"
diff -r examples/esp32-s31-devboard/fab "$work/fab"
echo "production files: as committed"

mkdir "$work/config"
SPARKSMITH_CONFIG_DIR=$work/config
SPARKSMITH_CACHE_DIR=$work/cache
SPARKSMITH_RECOVERY_DIR=$work/recovery
# A Windows program reads Windows paths.
if command -v cygpath >/dev/null 2>&1; then
    SPARKSMITH_CONFIG_DIR=$(cygpath -w "$work/config")
    SPARKSMITH_CACHE_DIR=$(cygpath -w "$work/cache")
    SPARKSMITH_RECOVERY_DIR=$(cygpath -w "$work/recovery")
fi
export SPARKSMITH_CONFIG_DIR SPARKSMITH_CACHE_DIR SPARKSMITH_RECOVERY_DIR
"$programs/sparksmith-desktop$x" "$work/esp32-s31-devboard.sparksmith" > "$work/app.log" 2>&1 &
app=$!
opened=0
# Two minutes for a machine that draws in software.
for _ in $(seq 60); do
    kill -0 "$app" 2>/dev/null || break
    if grep -rqs esp32-s31-devboard.sparksmith "$work/config"; then opened=1; break; fi
    sleep 2
done
alive=0
if [ "$opened" = 1 ]; then
    # It goes on running once the board is drawn.
    sleep 10
    kill -0 "$app" 2>/dev/null && alive=1
fi
kill "$app" 2>/dev/null || true
cat "$work/app.log"
if [ "$alive" != 1 ]; then
    echo "the app did not open the example (opened: $opened)" >&2
    exit 1
fi
echo "app: opened the example and went on running"
