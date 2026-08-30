// Every symbol the wasm module imports, one per line as `module.name`.
//
// Read from the module rather than from a list someone maintains: a host
// function added to the interpreter becomes an import automatically, and the
// embedders that must supply it do not update themselves. When one was missed,
// all 256 programs failed at once — a missing import is an *instantiation*
// failure, so nothing runs and nothing points at the cause.
import { readFileSync } from "node:fs";

const mod = new WebAssembly.Module(readFileSync(process.argv[2]));
for (const { module, name } of WebAssembly.Module.imports(mod)) {
    console.log(`${module}.${name}`);
}
