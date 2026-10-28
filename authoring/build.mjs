import * as esbuild from "esbuild";
import { Worker } from "node:worker_threads";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { mkdir, rename, writeFile } from "node:fs/promises";

const runtimeRoot = fileURLToPath(new URL("./", import.meta.url));

// Normalize before structured clone or JSON.stringify can erase authoring mistakes.
// Keep this function self-contained: it also runs inside the scene worker.
function serializableScene(scene) {
  const ancestors = new Set();
  function visit(value, location, owner = "") {
    const fail = detail => { throw new Error(`${owner ? `${owner}: ` : ""}${location} ${detail}`); };
    if (value === null || typeof value === "string" || typeof value === "boolean") return value;
    if (typeof value === "number") {
      if (!Number.isFinite(value)) fail(`must be a finite JSON number; received ${value}`);
      return value;
    }
    if (typeof value !== "object") fail(`contains unsupported ${typeof value}; scene values must be JSON data`);
    if (ancestors.has(value)) fail("contains a circular reference; scene values must be JSON data");
    const entries = Array.isArray(value) ? undefined : Object.entries(value);
    owner = entries?.find(([key, item]) => key === "id" && typeof item === "string")?.[1] ?? owner;
    if (!Array.isArray(value) && Object.getPrototypeOf(value) !== Object.prototype && Object.getPrototypeOf(value) !== null) {
      fail("must be a plain JSON object, not a class instance");
    }
    if (Object.getOwnPropertySymbols(value).some(key => Object.prototype.propertyIsEnumerable.call(value, key))) {
      fail("contains a symbol property that JSON cannot preserve");
    }
    ancestors.add(value);
    try {
      if (Array.isArray(value)) {
        if (Object.keys(value).some(key => !/^(0|[1-9]\d*)$/.test(key) || Number(key) >= value.length)) {
          fail("contains an array property that JSON cannot preserve");
        }
        return Array.from(value, (item, index) => visit(item, `${location}[${index}]`, owner));
      }
      // Explicitly undefined optional props have the same meaning as omitted props.
      return Object.fromEntries(entries.filter(([, item]) => item !== undefined)
        .map(([key, item]) => [key, visit(item, `${location}[${JSON.stringify(key)}]`, owner)]));
    } finally { ancestors.delete(value); }
  }
  return visit(scene, "scene");
}

export function buildOptions(entry) {
  return {
    entryPoints: [path.resolve(entry)], absWorkingDir: path.dirname(runtimeRoot),
    outfile: "scene.mjs", bundle: true, write: false, platform: "node", format: "esm",
    target: "node22", jsx: "automatic", jsxImportSource: "@lemma/authoring",
    sourcemap: "inline", logLevel: "silent",
    plugins: [{ name: "lemma-runtime", setup(build) {
      build.onResolve({ filter: /^@lemma\/authoring(?:\/jsx-runtime)?$/ }, args => ({
        path: path.join(runtimeRoot, args.path.endsWith("/jsx-runtime") ? "jsx-runtime.ts" : "index.ts"),
      }));
    } }],
  };
}

/** A short-lived worker bounds module-cache memory across arbitrary scene reloads. */
export function evaluateBundle(bundle, timeoutMs = 30_000) {
  return new Promise((resolve, reject) => {
    const runner = `import { parentPort, workerData } from 'node:worker_threads';
      const serializableScene = ${serializableScene.toString()};
      try {
        const module = await import('data:text/javascript;base64,' + Buffer.from(workerData).toString('base64'));
        const scene = typeof module.default === 'function' ? await module.default() : module.default;
        if (!scene || scene.version !== 1 || !Array.isArray(scene.nodes)) throw new Error('Default export must be a version 1 Composition or a function returning one');
        parentPort.postMessage({ok:true,scene:serializableScene(scene)});
      } catch(error) { parentPort.postMessage({ok:false,error:error.stack ?? String(error)}); }`;
    const worker = new Worker(new URL(`data:text/javascript,${encodeURIComponent(runner)}`), {
      workerData: bundle, execArgv: ["--enable-source-maps"],
    });
    const timer = setTimeout(() => { worker.terminate(); reject(new Error(`Scene construction exceeded ${timeoutMs} ms`)); }, timeoutMs);
    let settled = false;
    const finish = () => { settled = true; clearTimeout(timer); void worker.terminate(); };
    worker.once("message", message => { finish(); message.ok ? resolve(message.scene) : reject(new Error(message.error)); });
    worker.once("error", error => { finish(); reject(error); });
    worker.once("exit", code => { if (!settled) { clearTimeout(timer); reject(new Error(`Scene worker exited before returning a scene (code ${code})`)); } });
  });
}
export async function compileScene(entry) {
  const result = await esbuild.build(buildOptions(entry));
  return evaluateBundle(result.outputFiles[0].text);
}
export async function writeScene(output, scene, sourceDirectory) {
  const saved = serializableScene(scene);
  // Saved assets remain relative to the generated scene, even when output lives elsewhere.
  for (const asset of saved.assets ?? []) {
    if (asset.source.kind === "saved") {
      asset.source.manifest = path.relative(path.dirname(path.resolve(output)), path.resolve(sourceDirectory, asset.source.manifest));
    }
  }
  await mkdir(path.dirname(path.resolve(output)), { recursive: true });
  const temporary = `${output}.${process.pid}.tmp`;
  await writeFile(temporary, JSON.stringify(saved, null, 2) + "\n");
  await rename(temporary, output);
}
export { esbuild };
