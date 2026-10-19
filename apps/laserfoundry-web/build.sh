#!/bin/sh
# Builds the browser version into apps/laserfoundry-web/pkg. Needs the wasm target and a
# wasm-bindgen CLI matching the wasm-bindgen crate version in Cargo.lock:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version <version from Cargo.lock>
# Then serve this folder over HTTP (for example `python3 -m http.server`) and open index.html.
set -eu
cd "$(dirname "$0")/../.."
cargo build --release --target wasm32-unknown-unknown -p laserfoundry-web
wasm-bindgen --target web --no-typescript --out-dir apps/laserfoundry-web/pkg \
  target/wasm32-unknown-unknown/release/laserfoundry_web.wasm
echo "built apps/laserfoundry-web/pkg; serve apps/laserfoundry-web and open index.html"
