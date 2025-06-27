#!/usr/bin/env sh
# Builds the parser as WebAssembly and copies it next to the web page.
# Afterwards, web/ is a self-contained static site.
set -eu

cd "$(dirname "$0")"

cargo build --lib --release --target wasm32-unknown-unknown --no-default-features
cp target/wasm32-unknown-unknown/release/nczrs.wasm web/nczrs.wasm

echo "Built web/nczrs.wasm. Serve web/ with any static file server, e.g.:"
echo "  python3 -m http.server -d web 8000"
