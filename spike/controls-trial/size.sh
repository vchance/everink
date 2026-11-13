#!/bin/sh
# Builds the size probe for WebAssembly with no candidate, with each, and with both, and prints the sizes.
#
#   sh spike/controls-trial/size.sh
#
# Needs the `wasm32-unknown-unknown` target (rust-toolchain.toml has it). `brotli` is optional.
set -eu
cd "$(dirname "$0")"

echo "probe: bytes, gzip -9, brotli"
for features in "" own egui egui-fonts "own,egui-fonts"; do
  name=${features:-baseline}
  cargo build --quiet --locked --profile small --target wasm32-unknown-unknown --example size_probe \
    --no-default-features --features "$features" --target-dir "target/size-$name"
  f="target/size-$name/wasm32-unknown-unknown/small/examples/size_probe.wasm"
  if command -v brotli >/dev/null; then b=$(brotli -c "$f" | wc -c); else b="-"; fi
  echo "$name: $(wc -c <"$f") $(gzip -9 -c "$f" | wc -c) $b"
done
