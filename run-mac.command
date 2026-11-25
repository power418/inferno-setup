#!/bin/bash
# Double-click untuk menyalakan emulator + signing server di macOS.
#_arg: main | all | view | stop | accel-check | ...
DIR="$(cd "$(dirname "$0")" && pwd)"
if [ "$#" -eq 0 ]; then
    exec "$DIR/inferno-mac.command" all
fi
exec "$DIR/inferno-mac.command" "$@"
