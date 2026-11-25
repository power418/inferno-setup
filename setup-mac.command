#!/bin/bash
# Double-click untuk setup penuh di macOS. Bisa juga: ./setup-mac.command build
DIR="$(cd "$(dirname "$0")" && pwd)"
if [ "$#" -eq 0 ]; then
    exec "$DIR/inferno-mac.command" setup
fi
exec "$DIR/inferno-mac.command" "$@"
