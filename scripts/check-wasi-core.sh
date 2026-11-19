#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
work=$(mktemp -d "${TMPDIR:-/tmp}/interface-core-tests.XXXXXX")
trap 'rm -rf -- "$work"' EXIT
export LC_ALL=C
export PATH="$PWD/scripts:$PATH"
export CARGO_TARGET_WASM32_WASIP1_RUNNER=run-wasi-core.mjs
suite=(--release --locked -p alumina-interface-core --lib)

cargo test "${suite[@]}" -- --list > "$work/native"
sed -n 's/: test$//p' "$work/native" | sort > "$work/expected"
test -s "$work/expected"
cargo test "${suite[@]}" --target wasm32-wasip1 -- --test-threads=1 | tee "$work/wasi"
sed -n 's/^test \(.*\) \.\.\. ok$/\1/p' "$work/wasi" | sort > "$work/passed"
diff -u "$work/expected" "$work/passed"
