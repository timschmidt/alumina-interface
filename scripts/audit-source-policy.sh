#!/usr/bin/env bash
set -euo pipefail

audit_native_inventory="$(mktemp)"
audit_wasm_inventory="$(mktemp)"
audit_path_inverse="$(mktemp)"
trap 'rm -f -- "$audit_native_inventory" "$audit_wasm_inventory" "$audit_path_inverse"' EXIT

cargo tree --workspace --target x86_64-unknown-linux-gnu --offline \
  --prefix none --format '{p}|{l}' >"$audit_native_inventory"
cargo tree --workspace --target wasm32-unknown-unknown --offline \
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

for package in \
  csgrs \
  hypercurve \
  hypergraphics \
  hyperlattice \
  hyperlimit \
  hypermesh \
  hyperpath \
  hyperphysics \
  hyperreal \
  hypersolve \
  hypertri; do
  cargo tree --target wasm32-unknown-unknown --offline -i "$package" \
    --prefix none >"$audit_path_inverse"
  if ! rg -q "^${package} v[^ ]+ \(.*/${package}\)$" "$audit_path_inverse"; then
    echo "$package must resolve from the sibling workspace checkout" >&2
    exit 1
  fi
done

echo "source policy: local CSGRS/Hyper stack; native and WASM license inventories accepted"
