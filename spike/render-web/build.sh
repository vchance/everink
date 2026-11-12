#!/bin/sh
# Builds the browser trial into www/ and prints the size of each WebAssembly module.
#
#   sh spike/render-web/build.sh
#
# Needs the `wasm32-unknown-unknown` target (rust-toolchain.toml has it) and `wasm-bindgen` 0.2.130 on PATH, the
# version `Cargo.toml` pins: `cargo install wasm-bindgen-cli --version 0.2.130 --locked`. `brotli` is optional.
#
#   www/pkg        all three renderers, for the page
#   www/pkg-simd   the same with WebAssembly SIMD (`?pkg=pkg-simd`)
#   www/size-*     one renderer each, built for size, to read how much each adds
set -eu
cd "$(dirname "$0")"

build() { # name, features, profile, rustflags
  RUSTFLAGS="$4" cargo build --locked --profile "$3" --target wasm32-unknown-unknown --features "$2" \
    --target-dir "target/$1"
  wasm-bindgen --target web --out-dir "www/$1" "target/$1/wasm32-unknown-unknown/$3/render_web.wasm"
}

build pkg webgpu,webgl,cpu release ""
build pkg-simd webgpu,webgl,cpu release "-C target-feature=+simd128"
build size-webgpu webgpu small ""
build size-webgl webgl small ""
build size-cpu cpu small ""
build size-all webgpu,webgl,cpu small ""

echo
echo "module: bytes, gzip -9, brotli"
for d in pkg pkg-simd size-webgpu size-webgl size-cpu size-all; do
  f="www/$d/render_web_bg.wasm"
  if command -v brotli >/dev/null; then b=$(brotli -c "$f" | wc -c); else b="-"; fi
  echo "$d: $(wc -c <"$f") $(gzip -9 -c "$f" | wc -c) $b"
done
