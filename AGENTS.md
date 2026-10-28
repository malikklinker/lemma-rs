# Working on lemma-rs

## Direction

Build the agreed acceptance scene in `docs/rendering-target.md` and follow the
boundaries in `docs/implementation.md`. Deliver working, inspectable vertical
slices. Keep the design documents' proposed capabilities distinct from what the
code actually supports.

The user welcomes subagents for substantial independent tasks. Delegate work with
clear file ownership, communicate interface changes, and integrate and verify the
combined result. Do not have multiple agents edit the same file concurrently.

## Architecture

- `scene` is the versioned, serializable authoring boundary. Unknown fields and
  unsupported operations should produce actionable errors with stable node IDs.
- `eval` compiles/validates once and resolves a `FramePacket` at an explicit sample.
  Seeking must not depend on previous frame requests.
- `time` uses integer frames and positive rational frame rates. Do not accumulate
  floating-point frame deltas for scene semantics.
- `data` owns immutable numerical results, metadata validation, and content identity.
  Camera, palette, and annotation edits must reuse numerical results when inputs
  have not changed. Dense data belongs in binary assets, not per-frame JSON.
- Only `src/render/` imports wgpu types. One selected device owns rendering resources.
  Keep rendering/presentation separate from explicit capture and encoding.
- TSX components execute on scene load/reload. Tracks describe animation for Rust;
  do not imply that arbitrary JavaScript callbacks execute in shaders or Rust.
- Preview and export share scene evaluation and rendering.

Use existing modules when practical. Add abstractions when a working use case
needs them; avoid empty frameworks and unimplemented public promises.

## Verification

Run `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings`, and relevant
`cargo test` checks. For authoring changes run `npm run typecheck` and `npm test`.
Commit dependency lockfiles with implementation changes.

Tests should protect meaningful contracts: rational timing and endpoints,
malformed metadata/references, direct/shuffled seeks, source/cache identity,
reload failure recovery, and GPU depth/color/alpha behavior. Inspect saved PNGs
when changing rendering. Test a short export when changing delivery.

Never silently substitute a software adapter for an explicitly requested GPU.
Report the actual adapter and driver. Software Vulkan is useful for correctness;
it provides no evidence of RTX/AMD hardware throughput. Measure initialization,
evaluation, uploads, GPU waits/readback, and encoding separately.

## Development environment

The initial host is NixOS. Cargo/Node are available, but this tool shell may lack
GPU device nodes and the Vulkan loader on its default dynamic-library path.
Prefer `nix develop` and the repository's runtime scripts once available. Do not
change host drivers or system configuration as part of ordinary engine work.

In the current restricted tool environment, downloaded dependency caches live
under `/tmp/lemma-cargo` and `/tmp/lemma-npm-cache`. Use
`CARGO_HOME=/tmp/lemma-cargo cargo … --locked --offline` after dependencies have
been fetched. Do not hardcode these temporary paths into normal user workflows.

## Collaboration and repository hygiene

Preserve existing user edits, including the initial uncommitted design documents.
Keep generated frames/videos under ignored `out/`, caches under ignored cache
directories, and source examples under `examples/`. Do not commit or push unless
requested. Document current commands, limitations, and reproducible verification
evidence when a capability lands.
