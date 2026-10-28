import test from "node:test";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));
const cli = path.join(root, "authoring/cli.mjs");
const engine = path.resolve(process.env.LEMMA_ENGINE ?? path.join(root, "target/debug/lemma"));
const execute = (args, options = {}) => promisify(execFile)(process.execPath, [cli, ...args], {
  cwd: root, timeout: 10_000, ...options,
});
const scene = `import { Composition, Camera, Field, Surface, ocean } from '@lemma/authoring';
  export default () => <Composition duration={8}>
    <Camera eye={[0, 3, 5]} target={[0, 0, 0]} />
    <Field id="field" source={{ kind: 'synthetic', resolution: [3, 3], bounds: [-1, 1, -1, 1] }} />
    <Surface id="surface" field="field" height={1} palette={ocean} />
  </Composition>;`;

async function directory(t) {
  const result = await mkdtemp(path.join(tmpdir(), "lemma-cli-"));
  t.after(() => rm(result, { recursive: true, force: true }));
  await writeFile(path.join(result, "scene.tsx"), scene);
  return result;
}

test("authoring help explains inspect/render, output defaults, and sample flag scope", async () => {
  const { stdout, stderr } = await execute(["--help"]);
  assert.equal(stderr, "");
  assert.match(stdout, /npm run inspect/); assert.match(stdout, /npm run render/);
  assert.match(stdout, /default out\/frame\.png/); assert.match(stdout, /default out\/reference\.json/);
  assert.match(stdout, /same persistent Rust host/); assert.match(stdout, /sample flags are rejected/);
});

test("mode-specific options cannot silently be ignored", async t => {
  const cases = [
    ["build", ["--frame", "0"], /--frame is only supported/],
    ["watch", ["--fps", "60"], /--fps is only supported/],
    ["build", ["--size", "1280x720"], /--size is only supported/],
    ["watch", ["--adapter", "GPU"], /--adapter is only supported/],
    ["inspect", ["--allow-software"], /does not initialize a GPU/],
    ["inspect", ["--output", "out.json"], /shell redirection/],
    ["render", ["--paused"], /--paused is only supported by preview/],
    ["render", ["--capture-dir", "out/captures"], /--capture-dir is only supported by preview/],
  ];
  for (const [mode, flags, expected] of cases) await t.test(`${mode} ${flags.join(" ")}`, async () => {
    await assert.rejects(execute([mode, "missing.tsx", "--engine", "/nonexistent/lemma", ...flags]), error => {
      assert.equal(error.code, 1); assert.equal(error.stdout, "");
      assert.match(error.stderr, expected); assert.match(error.stderr, /Use --help/);
      assert.doesNotMatch(error.stderr, /Rust executable not found/);
      return true;
    });
  });
});

test("sample flags reject malformed frames, rates, and output sizes before starting a host", async t => {
  const cases = [
    ["--frame=-1", /--frame must/], ["--frame=1.5", /--frame must/],
    ["--frame=9007199254740992", /--frame must/],
    ["--fps=0", /--fps must/], ["--fps=30000\/0", /--fps must/],
    ["--fps=4294967296\/1", /--fps must/], ["--fps=29.97", /--fps must/],
    ["--size=0x720", /--size must/], ["--size=8193x1", /--size must/],
    ["--size=1280", /--size must/],
  ];
  for (const [flag, expected] of cases) await t.test(flag, async () => {
    await assert.rejects(execute(["inspect", "missing.tsx", "--engine", "/nonexistent/lemma", flag]), error => {
      assert.equal(error.code, 1); assert.match(error.stderr, expected); assert.match(error.stderr, /Use --help/);
      assert.doesNotMatch(error.stderr, /Rust executable not found/); return true;
    });
  });
});

test("inspect/render use one loaded host and route sample, GPU, policy, and output options", async t => {
  const fixture = `#!${process.execPath}
import { createInterface } from 'node:readline';
import { appendFileSync } from 'node:fs';
const log = value => appendFileSync(process.env.LEMMA_CLI_LOG, JSON.stringify({ pid: process.pid, ...value }) + '\\n');
log({ start: process.argv.slice(2) });
let loaded = false;
createInterface({ input: process.stdin }).on('line', line => {
  const request = JSON.parse(line); log({ request });
  let result = { revision: 1 };
  if (request.op === 'load-scene') loaded = true;
  if (request.op.endsWith('-sample')) {
    if (!loaded) throw new Error('sample requested before load');
    result = { sample: request, pid: process.pid };
  }
  process.stdout.write(JSON.stringify({ version: 1, id: request.id, ok: true, result }) + '\\n');
  if (request.op === 'shutdown') process.stdin.destroy();
});`;
  for (const [mode, output] of [["inspect"], ["render"], ["render", "custom/frame.png"]]) await t.test(`${mode}${output ? " with explicit output" : ""}`, async t => {
    const cwd = await directory(t);
    const executable = path.join(cwd, "engine.mjs"), log = path.join(cwd, "requests.jsonl");
    await writeFile(executable, fixture, { mode: 0o755 });
    const gpu = mode === "render" ? ["--adapter", "fixture GPU", "--backend", "gl", "--allow-software"] : [];
    const { stdout, stderr } = await execute([mode, "scene.tsx", "--engine", executable,
      "--frame", "240", "--fps", "30000/1001", "--size", "127X65", "--require-saved", ...gpu,
      ...(output ? ["--output", output] : [])], {
      cwd, env: { ...process.env, LEMMA_CLI_LOG: log },
    });
    assert.equal(stderr, "");
    const result = JSON.parse(stdout), events = (await readFile(log, "utf8")).trim().split("\n").map(JSON.parse);
    assert.equal(new Set(events.map(event => event.pid)).size, 1);
    assert.deepEqual(events[0].start, ["serve", ...gpu]);
    const requests = events.slice(1).map(event => event.request);
    assert.deepEqual(requests.map(request => request.op), ["load-scene", `${mode}-sample`, "shutdown"]);
    assert.equal(requests[0].policy, "require-saved"); assert.equal(requests[0].base_dir, cwd);
    assert.equal(result.sample.frame, 240); assert.deepEqual(result.sample.fps, { numerator: 30000, denominator: 1001 });
    assert.deepEqual(result.sample.size, [127, 65]);
    if (mode === "render") assert.equal(result.sample.output, path.join(cwd, output ?? "out/frame.png"));
    else assert.equal("output" in result.sample, false);
    assert.equal(existsSync(path.join(cwd, "out/reference.json")), false);
  });
});

test("empty explicit adapter names and output paths are rejected", async () => {
  for (const [flag, expected] of [["--adapter=", /nonempty adapter-name/], ["--output=", /nonempty file path/]]) {
    await assert.rejects(execute(["render", "--engine", "/nonexistent/lemma", flag]), error => {
      assert.equal(error.code, 1); assert.match(error.stderr, expected); return true;
    });
  }
});

test("build retains its JSON output default", {
  timeout: 15_000,
  skip: !process.env.LEMMA_ENGINE && !existsSync(engine) ? "Build the Rust CLI or set LEMMA_ENGINE to validate TSX" : false,
}, async t => {
  const cwd = await directory(t);
  const { stdout, stderr } = await execute(["build", "scene.tsx", "--engine", engine], { cwd });
  assert.equal(stdout, ""); assert.match(stderr, /Scene 1/);
  const spec = JSON.parse(await readFile(path.join(cwd, "out/reference.json"), "utf8"));
  assert.equal(spec.nodes[0].id, "surface"); assert.equal(spec.version, 1);
  assert.equal(existsSync(path.join(cwd, "out/frame.png")), false);
});

test("preview forwards sample, saved-only, GPU, and capture directory options", { timeout: 10_000 }, async t => {
  const cwd = await directory(t), executable = path.join(cwd, "engine.mjs"), log = path.join(cwd, "requests.jsonl");
  const fixture = `#!${process.execPath}
import { createInterface } from 'node:readline';
import { appendFileSync } from 'node:fs';
const log = value => appendFileSync(process.env.LEMMA_CLI_LOG, JSON.stringify(value) + '\\n');
log({ start: process.argv.slice(2) });
if (process.argv[2] === 'preview') process.exit(0);
createInterface({ input: process.stdin }).on('line', line => {
  const request = JSON.parse(line); log({ request });
  process.stdout.write(JSON.stringify({ version: 1, id: request.id, ok: true, result: { revision: 1 } }) + '\\n');
  if (request.op === 'shutdown') process.stdin.destroy();
});`;
  await writeFile(executable, fixture, { mode: 0o755 });
  const output = path.join(cwd, "published/scene.json"), captures = path.join(cwd, "captures");
  await execute(["preview", "scene.tsx", "--engine", executable, "--output", output,
    "--frame", "240", "--fps", "30000/1001", "--size", "640x360", "--paused", "--capture-dir", "captures",
    "--adapter", "fixture GPU", "--backend", "gl", "--allow-software", "--require-saved"], {
    cwd, env: { ...process.env, LEMMA_CLI_LOG: log },
  });
  const events = (await readFile(log, "utf8")).trim().split("\n").map(JSON.parse);
  assert.deepEqual(events.find(event => event.start?.[0] === "preview").start, [
    "preview", "--scene", output, "--frame", "240", "--adapter", "fixture GPU", "--backend", "gl", "--allow-software",
    "--size", "640x360", "--fps", "30000/1001", "--paused", "--capture-dir", captures, "--require-saved",
  ]);
  assert.equal(events.find(event => event.request?.op === "load-scene").request.policy, "require-saved");
  assert.equal(events.at(-1).request.op, "shutdown");
  assert.equal(JSON.parse(await readFile(output, "utf8")).version, 1);
});

test("real inspect resolves TSX without a GPU and saved-only errors emit no result", {
  timeout: 15_000,
  skip: !process.env.LEMMA_ENGINE && !existsSync(engine) ? "Build the Rust CLI or set LEMMA_ENGINE to inspect TSX" : false,
}, async t => {
  const cwd = await directory(t);
  const args = ["inspect", "scene.tsx", "--engine", engine, "--frame", "3", "--fps", "60000/2002", "--size", "127x65"];
  const { stdout, stderr } = await execute(args, { cwd });
  const result = JSON.parse(stdout);
  assert.equal(stderr, ""); assert.equal(result.revision, 1);
  assert.deepEqual(result.sample, { frame: 3, fps: { numerator: 30000, denominator: 1001 } });
  assert.deepEqual(result.size, [127, 65]); assert.equal(result.assets[0].id, "field");
  assert.equal(existsSync(path.join(cwd, "out")), false);
  await assert.rejects(execute([...args, "--require-saved"], { cwd }), error => {
    assert.equal(error.code, 1); assert.equal(error.stdout, ""); assert.match(error.stderr, /saved/); return true;
  });
});
