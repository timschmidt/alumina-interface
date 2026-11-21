#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
command -v rg >/dev/null || { echo "source policy: ripgrep (rg) is required" >&2; exit 1; }
python3 scripts/audit-sibling-sources.py "$@"

audit_native_inventory="$(mktemp)"
audit_wasm_inventory="$(mktemp)"
trap 'rm -f -- "$audit_native_inventory" "$audit_wasm_inventory"' EXIT

cargo tree --locked --workspace --target x86_64-unknown-linux-gnu --offline \
  --prefix none --format '{p}|{l}' >"$audit_native_inventory"
cargo tree --locked --workspace --target wasm32-unknown-unknown --offline \
  --prefix none --format '{p}|{l}' >"$audit_wasm_inventory"

if rg -ni '(^|[^A-Z])(AGPL|GPL|LGPL|SSPL)(-|[^A-Z]|$)' \
  "$audit_native_inventory" "$audit_wasm_inventory"; then
  echo "GPL-family dependency rejected" >&2
  exit 1
fi

if rg -n '\|$' "$audit_native_inventory" "$audit_wasm_inventory"; then
  echo "dependency with missing license metadata rejected" >&2
  exit 1
fi

echo "source policy: local Alumina/CSGRS/Hyper stacks; native and WASM license inventories accepted"
