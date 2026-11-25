#!/bin/bash
# Double-click untuk cek apakah akselerasi Inferno bisa dipakai di Mac ini.
DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$DIR/inferno-mac.command" accel-check "$@"
