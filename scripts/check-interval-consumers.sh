#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../../csgrs"
foc=crates/hyperlattice-interval/conformance/alumina-foc
measurement=crates/hyperinterval/conformance/alumina-measurements
# Give both linked WASM runners a known artifact path, including on CI.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
wasm=wasm32-unknown-unknown
cargo +1.88.0 test --locked --release --manifest-path "$foc/Cargo.toml"
cargo +1.88.0 check --locked --target thumbv7em-none-eabihf --lib --manifest-path "$foc/Cargo.toml"
cargo +1.88.0 build --locked --release --target "$wasm" --manifest-path "$foc/wasm/Cargo.toml"
node "$foc/wasm/check.mjs" "$CARGO_TARGET_DIR/$wasm/release/interval_foc_wasm_consumer.wasm"

cargo +1.95.0 test --locked --features trace --manifest-path "$measurement/Cargo.toml"
cargo +1.95.0 build --locked --release --target "$wasm" --manifest-path "$measurement/wasm/Cargo.toml"
node "$measurement/wasm/check.mjs" "$CARGO_TARGET_DIR/$wasm/release/interval_measurement_wasm.wasm"
