import test from "node:test";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { compileScene } from "../build.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
const engine = path.resolve(process.env.LEMMA_ENGINE ?? path.join(root, "target/debug/lemma"));

// Rust serializes defaults while TSX may omit them. Expand the public schema's
// defaults before comparing the two independent authoring fixtures.
function defaults(value) {
  if (Array.isArray(value)) return value.map(defaults);
  if (!value || typeof value !== "object") return value;
  const copy = Object.fromEntries(Object.entries(value).map(([key, item]) => [key, defaults(item)]));
  if (copy.version === 1 && Array.isArray(copy.nodes)) {
    copy.model = { translation: [0, 0, 0], rotation_degrees: [0, 0, 0], scale: 1, ...copy.model };
  }
  if (copy.keyframes) copy.keyframes = copy.keyframes.map(key => ({ ease: "linear", ...key }));
  if (copy.palette) copy.palette = { shift: 0, ...copy.palette };
  const nodeDefaults = {
    surface: { grid: 0 }, trace: { lift: 0, reveal: 1 }, polyline: { reveal: 1 },
    markers: { lift: 0 }, text: { color: [1, 1, 1, 1], opacity: 1 },
    label: { color: [1, 1, 1, 1], occlusion: "always" }, "graphics-plane": { color: [1, 1, 1, 1] },
    field: { lift: 0 },
  };
  return { ...(nodeDefaults[copy.kind] ?? {}), ...copy };
}

function equivalent(actual, expected, location = "scene") {
  if (typeof actual === "number" && typeof expected === "number") {
    // JS generates points with f64 trig; the Rust fixture uses f32 trig.
    assert.ok(Number.isFinite(actual) && Math.abs(actual - expected) <= 1e-6 * Math.max(1, Math.abs(expected)),
      `${location}: TSX ${actual} differs from Rust ${expected}`);
  } else if (Array.isArray(expected)) {
    assert.ok(Array.isArray(actual), `${location} must be an array`);
    assert.equal(actual.length, expected.length, `${location}.length`);
    expected.forEach((value, i) => equivalent(actual[i], value, `${location}[${i}]`));
  } else if (expected && typeof expected === "object") {
    assert.ok(actual && typeof actual === "object", `${location} must be an object`);
    assert.deepEqual(Object.keys(actual).sort(), Object.keys(expected).sort(), `${location} fields`);
    for (const key of Object.keys(expected)) equivalent(actual[key], expected[key], `${location}.${key}`);
  } else assert.equal(actual, expected, location);
}

test("TSX and Rust acceptance scenes agree after defaults and f32 rounding", {
  timeout: 15_000,
  skip: !process.env.LEMMA_ENGINE && !existsSync(engine) ? "Build the Rust CLI or set LEMMA_ENGINE to check cross-language parity" : false,
}, async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lemma-reference-parity-"));
  try {
    const output = path.join(directory, "reference.json");
    await promisify(execFile)(engine, ["dump-scene", "--output", output], { timeout: 10_000 });
    const rust = JSON.parse(await readFile(output, "utf8"));
    const tsx = await compileScene(path.join(root, "examples/reference.tsx"));
    equivalent(defaults(tsx), defaults(rust));
  } finally { await rm(directory, { recursive: true, force: true }); }
});
