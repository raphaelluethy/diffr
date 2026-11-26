#!/usr/bin/env bash
# Run the deterministic runtime benchmarks serially with a disposable build directory.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ -z ${CARGO_TARGET_DIR:-} ]]; then
    CARGO_TARGET_DIR=$(mktemp -d "${TMPDIR:-/tmp}/diffr-perf.XXXXXXXX")
    trap 'rm -rf -- "$CARGO_TARGET_DIR"' EXIT
fi
export CARGO_TARGET_DIR
export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
cargo test --locked -p diffr -p diffr-agent -j "${CARGO_BUILD_JOBS:-4}" "$@" \
    perf_ -- --ignored --nocapture --test-threads=1
