#!/bin/sh
# Builds the controls page's module into www/pkg.
#
#   sh spike/controls-web/build.sh
#
# Needs the `wasm32-unknown-unknown` target (rust-toolchain.toml has it) and `wasm-bindgen` 0.2.130 on PATH, the
# version `Cargo.toml` pins: `cargo install wasm-bindgen-cli --version 0.2.130 --locked`.
set -eu
cd "$(dirname "$0")"
cargo build --locked --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir www/pkg target/wasm32-unknown-unknown/release/controls_web.wasm
echo "built www/pkg: $(wc -c <www/pkg/controls_web_bg.wasm) bytes of wasm (both candidates, built for speed)"
