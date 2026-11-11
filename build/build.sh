#!/usr/bin/env sh
# Baut claude-tui im Release-Modus und legt das Binary in output/ ab.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
(cd "$root/source" && cargo build --release)
mkdir -p "$root/output"
cp "$root/source/target/release/claude-tui" "$root/output/"
echo "Fertig: $root/output/claude-tui"
