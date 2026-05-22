import assert from "node:assert/strict";
import { mkdtemp, mkdir, stat, utimes, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { assertFreshPkgWeb } from "./smoke_freshness.mjs";

test("assertFreshPkgWeb rejects pkg-web artifacts older than source files", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "auki-smoke-freshness-"));
  const pkgWeb = path.join(root, "pkg-web");
  const src = path.join(root, "src");
  await mkdir(pkgWeb);
  await mkdir(src);
  const artifact = path.join(pkgWeb, "auki_network_browser_wasm_bg.wasm");
  const source = path.join(src, "lib.rs");
  await writeFile(artifact, "wasm");
  await writeFile(source, "rust");

  const now = new Date();
  const old = new Date(now.getTime() - 10_000);
  await utimes(artifact, old, old);
  await utimes(source, now, now);

  await assert.rejects(
    () =>
      assertFreshPkgWeb({
        artifact,
        sources: [source],
        buildCommand: "wasm-pack build demo",
      }),
    /pkg-web artifacts are stale.*wasm-pack build demo/s,
  );
});

test("assertFreshPkgWeb accepts pkg-web artifacts newer than source files", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "auki-smoke-freshness-"));
  const pkgWeb = path.join(root, "pkg-web");
  const src = path.join(root, "src");
  await mkdir(pkgWeb);
  await mkdir(src);
  const artifact = path.join(pkgWeb, "auki_network_browser_wasm_bg.wasm");
  const source = path.join(src, "lib.rs");
  await writeFile(artifact, "wasm");
  await writeFile(source, "rust");

  const artifactTime = new Date();
  const sourceTime = new Date(artifactTime.getTime() - 10_000);
  await utimes(artifact, artifactTime, artifactTime);
  await utimes(source, sourceTime, sourceTime);

  await assert.doesNotReject(() =>
    assertFreshPkgWeb({
      artifact,
      sources: [source],
      buildCommand: "wasm-pack build demo",
    }),
  );

  assert((await stat(artifact)).mtimeMs > (await stat(source)).mtimeMs);
});
