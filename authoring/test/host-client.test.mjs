import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { once } from "node:events";
import { HostClient } from "../host-client.mjs";

const fixture = `
const { createInterface } = require('node:readline');
const mode = process.argv[2];
const queued = [];
const reply = response => process.stdout.write(JSON.stringify(response) + '\\n');
createInterface({ input: process.stdin }).on('line', line => {
  const request = JSON.parse(line);
  const response = { version: 1, id: request.id, ok: true, result: { op: request.op } };
  if (mode === 'exit') return process.exit(7);
  if (mode === 'hang' || mode === 'hang-shutdown' && request.op === 'shutdown') return;
  if (mode === 'invalid-json') return process.stdout.write('{broken\\n');
  if (mode === 'wrong-version') response.version = 2;
  if (mode === 'wrong-id') response.id = request.id + 1;
  if (mode === 'missing-result') delete response.result;
  if (mode === 'invalid-error') { response.ok = false; response.error = { message: 'missing code' }; delete response.result; }
  if (mode === 'array') return reply([]);
  if (request.op === 'fail') { response.ok = false; response.error = { code: 'scene-error', message: 'surface: missing field' }; delete response.result; }
  if (mode === 'reorder' && request.op !== 'shutdown') {
    queued.push(response);
    if (queued.length === 2) queued.splice(0).reverse().forEach(reply);
  } else reply(response);
  if (request.op === 'shutdown') process.exit(0);
});
`;

async function client(t, mode = "normal") {
  const directory = await mkdtemp(path.join(tmpdir(), "lemma-host-client-"));
  await writeFile(path.join(directory, "serve"), fixture);
  const host = new HostClient(process.execPath, [mode], { cwd: directory, stdio: ["pipe", "pipe", "pipe"] });
  t.after(async () => { await host.close().catch(() => {}); await rm(directory, { recursive: true, force: true }); });
  return host;
}

test("host correlates reordered responses and preserves structured scene errors", { timeout: 5_000 }, async t => {
  const host = await client(t, "reorder");
  const [first, second] = await Promise.all([host.request("first"), host.request("second")]);
  assert.equal(first.op, "first"); assert.equal(second.op, "second");
  const failed = host.request("fail");
  const next = host.request("info");
  await assert.rejects(failed, { code: "scene-error", message: "surface: missing field" });
  assert.equal((await next).op, "info");
  assert.equal(host.pending.size, 0);
});

test("malformed host output terminates the session and rejects future work", { timeout: 5_000 }, async t => {
  for (const mode of ["invalid-json", "wrong-version", "wrong-id", "missing-result", "invalid-error", "array"]) {
    await t.test(mode, async t => {
      const host = await client(t, mode);
      await assert.rejects(host.request("info"), /invalid (JSON|version 1 response)/);
      assert.equal(host.closed, true); assert.equal(host.pending.size, 0);
      await assert.rejects(host.request("info"), /invalid (JSON|version 1 response)/);
    });
  }
});

test("local serialization failures keep the session usable and protect its envelope", { timeout: 5_000 }, async t => {
  const host = await client(t);
  const circular = {}; circular.self = circular;
  await assert.rejects(host.request("info", circular), /circular/i);
  assert.equal(host.pending.size, 0);
  assert.equal((await host.request("info", { version: 99, id: 99, op: "fail" })).op, "info");
});

test("spawn and early exit failures promptly reject pending work", { timeout: 5_000 }, async t => {
  const missing = new HostClient(path.join(tmpdir(), "lemma-nonexistent-executable"));
  await assert.rejects(missing.request("info"), { code: "ENOENT" });
  await missing.close();
  const host = await client(t, "exit");
  await assert.rejects(host.request("info"), /Rust host (exited|closed)/);
  await assert.rejects(host.request("info"), /Rust host (exited|closed)/);
});

test("a timeout fails all pending requests and kills the host", { timeout: 5_000 }, async t => {
  const host = await client(t, "hang");
  const exited = once(host.process, "exit");
  const first = host.request("load-scene", {}, 100);
  const second = host.request("info");
  await Promise.all([
    assert.rejects(first, /timed out during load-scene/),
    assert.rejects(second, /timed out during load-scene/),
  ]);
  assert.equal(host.pending.size, 0);
  await exited;
  await host.close();
});

test("shutdown is idempotent and bounded even when the host stops responding", { timeout: 5_000 }, async t => {
  const host = await client(t, "hang-shutdown");
  await host.request("info");
  const closing = host.close(100);
  assert.equal(host.close(), closing);
  await assert.rejects(host.request("info"), /closing/);
  await assert.rejects(closing, /timed out during shutdown/);
  assert.equal(host.closed, true);
});
