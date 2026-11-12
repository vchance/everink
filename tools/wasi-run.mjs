#!/usr/bin/env node
// Runs a wasm32-wasip1 test or example binary under Node's WASI, so that `cargo test --target wasm32-wasip1`
// works with no tool beyond Node. Cargo calls it as the target's runner (.cargo/config.toml).
// The whole file system is visible to the module: the golden-file tests read their files by absolute path.
import { WASI } from 'node:wasi';
import { readFile } from 'node:fs/promises';
import { argv, env, exit } from 'node:process';

const [, , wasm, ...args] = argv;
if (!wasm) {
  console.error('usage: wasi-run.mjs <module.wasm> [args...]');
  exit(2);
}
const wasi = new WASI({
  version: 'preview1',
  args: [wasm, ...args],
  env,
  preopens: { '/': '/' },
  returnOnExit: true,
});
const module = await WebAssembly.compile(await readFile(wasm));
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
exit(wasi.start(instance));
