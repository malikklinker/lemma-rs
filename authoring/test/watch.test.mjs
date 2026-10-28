import test from "node:test";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));
const engine = path.resolve(process.env.LEMMA_ENGINE ?? path.join(root, "target/debug/lemma"));

test("watch retains the last valid scene across construction and validation errors, then recovers", {
  timeout: 15_000,
  skip: !process.env.LEMMA_ENGINE && !existsSync(engine) ? "Build the Rust CLI or set LEMMA_ENGINE to test persistent reloads" : false,
}, async t => {
  const directory = await mkdtemp(path.join(tmpdir(), "lemma-watch-"));
  const entry = path.join(directory, "scene.tsx");
  const output = path.join(directory, "out", "scene.json");
  const source = node => `import { Composition, Camera, Text, Trace } from '@lemma/authoring';
    export default () => <Composition duration={2}>
      <Camera eye={[0, 0, 5]} target={[0, 0, 0]} />${node}
    </Composition>;`;
  const text = value => `<Text id="note" position={[10, 10]} font_size={20}>${value}</Text>`;
  await writeFile(entry, source(text("initial valid")));
  const child = spawn(process.execPath, [path.join(root, "authoring/cli.mjs"), "watch", entry, "--output", output, "--engine", engine], {
    cwd: root, stdio: ["ignore", "pipe", "pipe"],
  });
  let stderr = "", cursor = 0, notify, stopped;
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", chunk => { stderr += chunk; notify?.(); });
  const exited = new Promise((resolve, reject) => {
    child.once("error", error => { stopped = error; notify?.(); reject(error); });
    child.once("exit", (code, signal) => { stopped = { code, signal }; notify?.(); resolve(stopped); });
  });
  // Handle unexpected exits immediately; the explicit await below still checks it.
  void exited.catch(() => {});
  t.after(async () => {
    if (!stopped) child.kill("SIGKILL");
    await exited.catch(() => {});
    await rm(directory, { recursive: true, force: true });
  });
  const waitFor = pattern => new Promise((resolve, reject) => {
    const timer = setTimeout(() => { notify = undefined; reject(new Error(`Watch did not emit ${pattern}:\n${stderr}`)); }, 5_000);
    notify = () => {
      const match = pattern.exec(stderr.slice(cursor));
      if (match) {
        cursor += match.index + match[0].length;
        clearTimeout(timer); notify = undefined; resolve();
      } else if (stopped) {
        clearTimeout(timer); notify = undefined; reject(new Error(`Watch stopped before ${pattern}:\n${stderr}`));
      }
    };
    notify();
  });

  await waitFor(/Scene 1 →/u);
  const original = await readFile(output, "utf8");
  assert.equal(JSON.parse(original).nodes[0].text, "initial valid");

  await writeFile(entry, "export default () => { throw new Error('construction exploded') }");
  await waitFor(/Scene unchanged: .*construction exploded/);
  assert.equal(await readFile(output, "utf8"), original);

  await writeFile(entry, source(`<Trace id="broken-trace" field="missing-field" points={[[-1, 0], [1, 0]]} height={0} width={2} color={[1, 1, 1, 1]} />`));
  await waitFor(/Scene unchanged: .*broken-trace/);
  assert.equal(await readFile(output, "utf8"), original);

  await writeFile(entry, source(text("recovered valid")));
  await waitFor(/Scene 2 →/u);
  const recovered = await readFile(output, "utf8");
  assert.equal(JSON.parse(recovered).nodes[0].text, "recovered valid");
  child.kill("SIGINT");
  const timer = setTimeout(() => child.kill("SIGKILL"), 3_000);
  try { assert.deepEqual(await exited, { code: 0, signal: null }); }
  finally { clearTimeout(timer); }
  assert.equal(await readFile(output, "utf8"), recovered);
});
