#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { WASI } from "node:wasi";

// Cargo passes the linked libtest binary followed by its arguments. The core
// links wasm-bindgen through its graphics dependency, even for a WASI command.
const [binary, ...args] = process.argv.slice(2);
const directory = await mkdtemp(path.join(tmpdir(), "interface-wasi-"));
try {
    execFileSync("wasm-bindgen", [
        binary, "--target", "bundler", "--no-typescript",
        "--out-name", "interface", "--out-dir", directory,
    ], { stdio: "inherit" });
    await writeFile(path.join(directory, "package.json"), '{"type":"module"}');
    const glue = await import(pathToFileURL(path.join(directory, "interface_bg.js")));
    const wasi = new WASI({ version: "preview1", args: [binary, ...args], returnOnExit: true });
    const module = await WebAssembly.compile(await readFile(path.join(directory, "interface_bg.wasm")));
    const instance = await WebAssembly.instantiate(module, {
        ...wasi.getImportObject(),
        "./interface_bg.js": glue,
    });
    glue.__wbg_set_wasm(instance.exports);
    instance.exports.__wbindgen_start?.();
    process.exitCode = wasi.start(instance);
} finally {
    await rm(directory, { recursive: true, force: true });
}
