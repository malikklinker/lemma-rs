import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { compileScene, evaluateBundle, writeScene } from "../build.mjs";

test("reference TSX constructs a declarative, serializable scene", async () => {
  const scene = await compileScene("examples/reference.tsx");
  assert.equal(scene.version, 1);
  assert.equal(scene.nodes.find(n => n.id === "title").text, "A field, given form");
  assert.equal(scene.camera.viewport[3], 452);
  const surface = scene.nodes.find(n => n.kind === "surface");
  assert.equal(surface.height.keyframes[1].value, 1.55);
  assert.deepEqual(scene.model.rotation_degrees.keyframes[1].value, [0, 12, 0]);
  assert.equal(scene.model.scale.keyframes[1].value, 0.96);
  assert.deepEqual(JSON.parse(JSON.stringify(scene)), scene);
  assert.deepEqual(await compileScene("examples/reference.tsx"), scene);
});

test("Compositions omit an unused identity model and preserve explicit partial transforms", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lemma-transform-"));
  try {
    const entry = path.join(directory, "scene.tsx");
    const source = model => `import { Composition, Camera } from '@lemma/authoring';
      export default () => <Composition duration={1} ${model}>
        <Camera eye={[0, 0, 5]} target={[0, 0, 0]} />
        {false}{null}{undefined}
      </Composition>;`;
    await writeFile(entry, source(""));
    assert.equal("model" in await compileScene(entry), false);
    await writeFile(entry, source("model={{ translation: [1, 2, 3], scale: 2 }}"));
    const scene = await compileScene(entry);
    assert.deepEqual(scene.model, { translation: [1, 2, 3], scale: 2 });
    assert.deepEqual(JSON.parse(JSON.stringify(scene)), scene);
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test("unsupported component results and raw Composition text produce actionable errors", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lemma-invalid-child-"));
  try {
    const entry = path.join(directory, "scene.tsx");
    const source = child => `import { Composition, Camera } from '@lemma/authoring';
      const Unsupported = () => ({ category: 'group', spec: { id: 'unsupported-group' } });
      export default () => <Composition duration={1}>
        <Camera eye={[0, 0, 5]} target={[0, 0, 0]} />${child}
      </Composition>;`;
    await writeFile(entry, source("<Unsupported />"));
    await assert.rejects(compileScene(entry), /unsupported-group: unsupported Composition child/);
    await writeFile(entry, source("raw child"));
    await assert.rejects(compileScene(entry), /put text inside a Text component/);
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test("authoring errors are reported, and a hung construction can be terminated", async () => {
  await assert.rejects(evaluateBundle("export default () => { throw new Error('bad camera') }"), /bad camera/);
  await assert.rejects(evaluateBundle("export default () => { while(true) {} }", 100), /exceeded/);
  await assert.rejects(evaluateBundle("export default { nodes: [] }"), /version 1 Composition/);
});

test("scene construction rejects values JSON would silently alter, with their owning node ID", async () => {
  for (const [expression, diagnostic] of [
    ["NaN", /finite JSON number; received NaN/],
    ["Infinity", /finite JSON number; received Infinity/],
    ["1n", /unsupported bigint/],
    ["() => 1", /unsupported function/],
    ["Symbol('callback')", /unsupported symbol/],
    ["new Date()", /plain JSON object/],
    ["new Map()", /plain JSON object/],
    ["new Set()", /plain JSON object/],
    ["new Float32Array([1, 2])", /plain JSON object/],
    ["(() => { const value = {}; value.self = value; return value; })()", /circular reference/],
    ["[1, , 3]", /unsupported undefined/],
    ["Object.assign([1], { extra: 2 })", /array property/],
    ["{ [Symbol('hidden')]: 1 }", /symbol property/],
  ]) {
    await assert.rejects(evaluateBundle(`export default { version: 1, nodes: [
      { id: 'broken-node', value: ${expression} }
    ] }`), error => {
      assert.match(error.message, /broken-node: scene\["nodes"\]\[0\]\["value"\]/);
      assert.match(error.message, diagnostic);
      return true;
    });
  }
});

test("serializable scene normalization preserves shared JSON values without treating them as cycles", async () => {
  const scene = await evaluateBundle(`const point = [1, 2, 3];
    export default { version: 1, nodes: [{ id: 'line', points: [point, point] }] };`);
  assert.deepEqual(scene.nodes[0].points, [[1, 2, 3], [1, 2, 3]]);
  assert.deepEqual(JSON.parse(JSON.stringify(scene)), scene);
});

test("explicitly undefined optional authoring props normalize to absent JSON fields", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lemma-optional-props-"));
  try {
    const entry = path.join(directory, "scene.tsx");
    await writeFile(entry, `import { Composition, Camera, Text } from '@lemma/authoring';
      export default () => <Composition duration={1} model={{ scale: undefined }}>
        <Camera eye={[0, 0, 5]} target={[0, 0, 0]} viewport={undefined} />
        <Text id="optional-text" position={[0, 0]} font_size={20} opacity={undefined}>Hello</Text>
      </Composition>;`);
    const scene = await compileScene(entry);
    assert.deepEqual(scene.model, {});
    assert.equal("viewport" in scene.camera, false);
    assert.equal("opacity" in scene.nodes[0], false);
    assert.deepEqual(JSON.parse(JSON.stringify(scene)), scene);
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test("writeScene rejects serialization loss before replacing a previous valid scene", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lemma-serialization-"));
  try {
    const output = path.join(directory, "scene.json");
    const scene = { assets: [], nodes: [{ id: "valid", value: 1 }] };
    await writeScene(output, scene, directory);
    const previous = await readFile(output, "utf8");
    scene.nodes[0].value = Infinity;
    await assert.rejects(writeScene(output, scene, directory), /valid: .*finite JSON number/);
    assert.equal(await readFile(output, "utf8"), previous);
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test("saved asset paths follow the generated scene location", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lemma-authoring-"));
  try {
    const source = path.join(directory, "source");
    const output = path.join(directory, "out", "scene.json");
    const scene = { assets: [{ id: "field", source: { kind: "saved", manifest: "data/field.json" } }], nodes: [] };
    await writeScene(output, scene, source);
    const saved = JSON.parse(await readFile(output, "utf8"));
    assert.equal(path.resolve(path.dirname(output), saved.assets[0].source.manifest), path.join(source, "data/field.json"));
    assert.equal(scene.assets[0].source.manifest, "data/field.json");
    assert.equal((await readFile(output, "utf8")).endsWith("\n"), true);
  } finally { await rm(directory, { recursive: true, force: true }); }
});
