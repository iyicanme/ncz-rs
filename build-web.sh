#!/usr/bin/env sh
# Builds the parser as WebAssembly with its JavaScript bindings into web/pkg.
# Afterwards, web/ is a self-contained static site.
set -eu

cd "$(dirname "$0")"

# The wasm-bindgen CLI must match the wasm-bindgen crate version exactly.
version=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"$/\1/p')
if ! wasm-bindgen --version 2>/dev/null | grep -qx "wasm-bindgen $version"; then
  echo "wasm-bindgen $version is required, install it with:" >&2
  echo "  cargo install wasm-bindgen-cli --version $version" >&2
  exit 1
fi

cargo build --lib --release --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir web/pkg target/wasm32-unknown-unknown/release/nczrs.wasm

echo "Built web/pkg. Serve web/ with any static file server, e.g.:"
echo "  python3 -m http.server -d web 8000"
