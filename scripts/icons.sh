#!/usr/bin/env bash
#
# Writes the icon files in assets/ (PNG, ICO, ICNS) from the drawing code in apps/mqtt-viewer/src/icon.rs.
# Run it after changing the icon; a test fails while the files are outdated.
#
# Usage: scripts/icons.sh

set -euo pipefail

cd "$(dirname "$0")/.."
MQTT_VIEWER_UPDATE_ICONS=1 cargo test -p mqtt-viewer the_icon_files_match_the_drawing
echo "Icons written to assets/."
