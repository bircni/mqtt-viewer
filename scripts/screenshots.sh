#!/usr/bin/env bash
#
# Renders the screenshots shown in the README into docs/. Needs a machine with a GPU.
#
# Usage: scripts/screenshots.sh

set -euo pipefail

cd "$(dirname "$0")/.."
cargo test -p mqtt-viewer screenshots -- --ignored
mkdir -p docs
for name in browse connect settings; do
    cp "target/screenshots/${name}.png" "docs/${name}.png"
done
echo "Screenshots written to docs/."
