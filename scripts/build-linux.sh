#!/bin/sh
# Native Linux build; derive runtime-library locations from the host linker cache.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
links="$root/target/native-link"
mkdir -p "$links"
for lib in xkbcommon xkbcommon-x11; do
    runtime=$(ldconfig -p | awk -v name="lib$lib.so.0" '$1 == name { print $NF; exit }')
    if test -z "$runtime" || ! test -f "$runtime"; then
        echo "Missing lib$lib; install the xkbcommon development packages." >&2
        exit 1
    fi
    ln -sf "$runtime" "$links/lib$lib.so"
done
export LIBRARY_PATH="$links${LIBRARY_PATH:+:$LIBRARY_PATH}"
exec cargo build --workspace "$@"
