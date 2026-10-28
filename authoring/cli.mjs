#!/usr/bin/env node
import { parseArgs } from "node:util";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";
import { buildOptions, compileScene, evaluateBundle, writeScene, esbuild } from "./build.mjs";
import { HostClient } from "./host-client.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const help = `Usage: node authoring/cli.mjs <build|watch|inspect|render|preview> [scene.tsx] [OPTIONS]

  build              Validate and write scene JSON (npm run scene)
  watch              Validate edits and publish scene JSON (npm run watch)
  inspect            Print resolved sample JSON without a GPU (npm run inspect)
  render             Capture a PNG and print its JSON report (npm run render)
  preview            Watch edits in a native GPU window (npm run preview)

  --output, -o PATH   JSON for build/watch/preview (default out/reference.json)
                     PNG for render (default out/frame.png); inspect uses stdout
  --engine PATH      Rust executable (default target/debug/lemma; build it first)
  --require-saved    Reject sources that need numerical computation
  --frame N          Nonnegative integer frame (inspect/render/preview; default 0)
  --fps N[/D]        Positive rational rate override (inspect/render/preview)
  --size WxH         Output dimensions in 1..=8192 (inspect/render/preview)
  --adapter NAME     Explicit adapter-name substring (render/preview)
  --backend NAME     vulkan, gl, or all (render/preview; default vulkan)
  --allow-software   Explicitly allow a software adapter (render/preview)
  --paused           Start preview paused
  --capture-dir PATH Preview's S-key PNG directory (default out/preview)

The default entry is examples/reference.tsx. Scene tracks run in Rust.
Inspect/render load and sample the scene in the same persistent Rust host.
Watch/preview keep the last valid scene after authoring or validation errors.
Build/watch fps and size come from Composition; sample flags are rejected there.`;

function parseFrame(value = "0") {
  if (!/^\d+$/.test(value) || !Number.isSafeInteger(Number(value))) {
    throw new Error("--frame must be a nonnegative integer no greater than 9007199254740991; use the Rust CLI for larger frame indices");
  }
  return Number(value);
}
function parseRate(value) {
  const parts = /^\d+(?:\/\d+)?$/.test(value) ? value.split("/").map(Number) : [];
  if (!parts.length || parts.some(part => !Number.isSafeInteger(part) || part < 1 || part > 0xffff_ffff)) {
    throw new Error("--fps must be a positive integer rate or N/D, with numerator and denominator in 1..=4294967295 (for example 30000/1001)");
  }
  return { numerator: parts[0], denominator: parts[1] ?? 1 };
}
function parseSize(value) {
  const parts = /^\d+x\d+$/i.test(value) ? value.toLowerCase().split("x").map(Number) : [];
  if (parts.length !== 2 || parts.some(part => !Number.isInteger(part) || part < 1 || part > 8192)) {
    throw new Error("--size must be WIDTHxHEIGHT with both dimensions in 1..=8192 (for example 1280x720)");
  }
  return parts;
}

async function main() {
  const { values, positionals } = parseArgs({ allowPositionals: true, options: {
    output: { type: "string", short: "o" },
    engine: { type: "string" }, adapter: { type: "string" }, backend: { type: "string" },
    "allow-software": { type: "boolean" }, "require-saved": { type: "boolean" }, frame: { type: "string" },
    size: { type: "string" }, fps: { type: "string" }, paused: { type: "boolean", default: false },
    "capture-dir": { type: "string" },
    help: { type: "boolean", short: "h", default: false },
  } });
  const [mode = "build", entry = "examples/reference.tsx"] = positionals;
  if (values.help) { console.log(help); return; }
  if (!["build", "watch", "inspect", "render", "preview"].includes(mode)) throw new Error(`Unknown command ${mode}`);
  if (positionals.length > 2) throw new Error("Provide one scene entry after the command; extra positional arguments are not supported");
  for (const flag of ["frame", "fps", "size"]) {
    if (values[flag] !== undefined && !["inspect", "render", "preview"].includes(mode)) {
      throw new Error(`--${flag} is only supported by inspect/render/preview; set Composition properties for ${mode}`);
    }
  }
  for (const flag of ["adapter", "backend", "allow-software"]) {
    if (values[flag] !== undefined && !["render", "preview"].includes(mode)) {
      throw new Error(`--${flag} is only supported by render/preview; ${mode} does not initialize a GPU`);
    }
  }
  if (values.paused && mode !== "preview") throw new Error("--paused is only supported by preview");
  if (values["capture-dir"] !== undefined && mode !== "preview") throw new Error("--capture-dir is only supported by preview");
  if (values["capture-dir"] === "") throw new Error("--capture-dir must be a nonempty directory path");
  if (values.output !== undefined && mode === "inspect") throw new Error("inspect prints JSON to stdout; use shell redirection instead of --output");
  if (values.output === "") throw new Error("--output must be a nonempty file path");
  if (values.adapter !== undefined && !values.adapter.trim()) throw new Error("--adapter must be a nonempty adapter-name substring");
  if (values.backend !== undefined && !["vulkan", "gl", "all"].includes(values.backend)) throw new Error("--backend must be vulkan, gl, or all");
  const sample = { frame: parseFrame(values.frame), ...(values.fps === undefined ? {} : { fps: parseRate(values.fps) }),
    ...(values.size === undefined ? {} : { size: parseSize(values.size) }) };
  const executable = path.resolve(values.engine ?? path.join(root, "target/debug/lemma"));
  if (!existsSync(executable)) throw new Error(`Rust executable not found at ${executable}. Run cargo build first, or pass --engine PATH.`);
  const gpuArgs = [];
  if (values.adapter) gpuArgs.push("--adapter", values.adapter);
  if (values.backend) gpuArgs.push("--backend", values.backend);
  if (values["allow-software"]) gpuArgs.push("--allow-software");
  const host = new HostClient(executable, gpuArgs);
  const output = path.resolve(values.output ?? (mode === "render" ? "out/frame.png" : "out/reference.json"));
  const sourceDirectory = path.dirname(path.resolve(entry));
  let context, preview, closing = false;
  async function shutdown(code = 0) {
    if (closing) return;
    closing = true;
    preview?.kill(); await context?.dispose(); await host.close().catch(() => {});
    process.exitCode = code;
  }
  process.once("SIGINT", () => { void shutdown(); });
  process.once("SIGTERM", () => { void shutdown(); });
  const load = scene => host.request("load-scene", { scene, base_dir: sourceDirectory,
    ...(values["require-saved"] ? { policy: "require-saved" } : {}) });
  async function publish(scene) {
    const result = await load(scene);
    await writeScene(output, scene, sourceDirectory);
    console.error(`Scene ${result.revision ?? "loaded"} → ${path.relative(process.cwd(), output)} (${scene.nodes.length} nodes)`);
    if (mode === "preview" && !preview) launchPreview();
  }
  function launchPreview() {
    const args = ["preview", "--scene", output, "--frame", String(sample.frame), ...gpuArgs];
    if (sample.size) args.push("--size", sample.size.join("x"));
    if (sample.fps) args.push("--fps", `${sample.fps.numerator}/${sample.fps.denominator}`);
    if (values.paused) args.push("--paused");
    if (values["capture-dir"]) args.push("--capture-dir", path.resolve(values["capture-dir"]));
    if (values["require-saved"]) args.push("--require-saved");
    preview = spawn(executable, args, { stdio: "inherit" });
    preview.once("error", error => { console.error(error.message); void shutdown(1); });
    preview.once("exit", code => { void shutdown(code ?? 1); });
  }
  host.process.once("exit", (code, signal) => {
    if (!closing) { console.error(`Rust host stopped (${signal ?? code})`); void shutdown(1); }
  });
  try {
    if (["build", "inspect", "render"].includes(mode)) {
      const scene = await compileScene(entry);
      if (mode === "build") await publish(scene);
      else {
        await load(scene);
        const result = await host.request(`${mode === "inspect" ? "inspect" : "render"}-sample`, {
          ...sample, ...(mode === "render" ? { output } : {}),
        });
        console.log(JSON.stringify(result, null, 2));
      }
      await shutdown();
    }
    else {
      const options = buildOptions(entry);
      options.plugins.push({ name: "publish-valid-scene", setup(build) {
        build.onEnd(async result => {
          if (closing) return;
          try {
            if (result.errors.length) {
              console.error((await esbuild.formatMessages(result.errors, { kind: "error", color: process.stderr.isTTY })).join("\n"));
            } else await publish(await evaluateBundle(result.outputFiles[0].text));
          } catch (error) { console.error(`Scene unchanged: ${error.message}`); }
        });
      } });
      context = await esbuild.context(options);
      await context.watch();
      console.error(`Watching ${entry}. Ctrl+C to stop.`);
    }
  } catch (error) {
    console.error(error.message); await shutdown(1);
  }
}

try { await main(); }
catch (error) { console.error(`${error.message}\nUse --help for commands and supported options.`); process.exitCode = 1; }
