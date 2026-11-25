#!/bin/sh
# Puts Sparksmith into PREFIX (default ~/.local), with its menu entry and the
# .sparksmith file type; or takes it out again. Settings and recovery files
# stay either way (README.txt names them).
#   sh install.sh [PREFIX]
#   sh install.sh --uninstall [PREFIX]
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
remove=0
if [ "${1:-}" = "--uninstall" ]; then remove=1; shift; fi
prefix=${1:-"$HOME/.local"}

if [ "$remove" = 1 ]; then
    rm -f "$prefix/bin/sparksmith" "$prefix/bin/sparksmith-desktop" \
        "$prefix/share/applications/io.sparksmith.app.desktop" \
        "$prefix/share/mime/packages/io.sparksmith.app.xml" \
        "$prefix/share/icons/hicolor/scalable/apps/io.sparksmith.app.svg"
    rm -rf "$prefix/share/sparksmith" "$prefix/share/doc/sparksmith"
else
    mkdir -p "$prefix/bin" "$prefix/share"
    # A whole new library: nothing of an earlier version stays in it.
    rm -rf "$prefix/share/sparksmith" "$prefix/share/doc/sparksmith"
    cp "$here/bin/sparksmith" "$here/bin/sparksmith-desktop" "$prefix/bin/"
    cp -R "$here/share/." "$prefix/share/"
    # The menu may not have PREFIX/bin on its path.
    sed "s|^Exec=.*|Exec=\"$prefix/bin/sparksmith-desktop\" %f|" \
        "$here/share/applications/io.sparksmith.app.desktop" \
        > "$prefix/share/applications/io.sparksmith.app.desktop"
fi
# Each is optional: a desktop without it has no such cache.
command -v update-mime-database >/dev/null 2>&1 && update-mime-database "$prefix/share/mime" || true
command -v update-desktop-database >/dev/null 2>&1 \
    && update-desktop-database "$prefix/share/applications" || true
if [ "$remove" = 1 ]; then
    echo "Sparksmith is removed from $prefix"
else
    echo "Sparksmith is in $prefix/bin: sparksmith-desktop (the app) and sparksmith (the command line)"
fi
