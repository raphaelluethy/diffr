#!/usr/bin/env bash
# Collects the licenses of every crate diffr links into one HTML file:
#   scripts/licenses.sh [<out.html>]
# Default: target/THIRD_PARTY_LICENSES.html. Fails when a crate's license is
# not among those accepted in about.toml.
# Needs cargo-about (cargo install --locked cargo-about).
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=${1:-target/THIRD_PARTY_LICENSES.html}
mkdir -p "$(dirname "$0")/../target" "$(dirname "$OUT")"
cargo about generate about.hbs -m crates/app/Cargo.toml -o "$OUT"
echo "Wrote $OUT"
