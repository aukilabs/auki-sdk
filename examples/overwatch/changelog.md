# Changelog - examples/overwatch

Append-only changelog for the Overwatch browser SDK example.

Latest entry on top.

---

### Nils's codex · May 27, HKT, 2026

**Raw camera preview compatibility restored.** Camera-kind previews now keep already-JPEG payloads on the raw path and skip malformed generated-protobuf camera frames without throwing through the preview subscription loop, while native `CameraFrame.frame` decoding remains covered.

Tests: `npm --prefix examples/overwatch run typecheck -- --pretty false`; `npm --prefix examples/overwatch test -- preview.test.ts streamHub.test.ts`; `npm --prefix examples/overwatch run build`; `git diff --check`.

### Nils's codex · May 27, HKT, 2026

**Native camera stream frames decode through generated protobuf bindings.** Overwatch now stages generated `@aukilabs/auki-proto`, declares the proto/runtime dependencies, tags runtime stream frames with matching sensor metadata, and decodes camera `CameraFrame.frame` bytes before creating JPEG preview blobs while preserving raw JPEG previews for non-camera streams.

Tests: `scripts/generate-javascript-proto.sh`; `npm --prefix examples/overwatch install`; `npm --prefix examples/overwatch test -- preview.test.ts streamHub.test.ts`; `npm --prefix examples/overwatch run build`.

### Nils's codex · May 26, HKT, 2026

**Park brand assets copied into Overwatch.** Overwatch now ships the `/brand/auki-monogram-white.svg` and `/brand/auki-wordmark-white.svg` files that Park's copied topbar references, with a focused test guarding the Vite public asset paths.

Tests: `npm --prefix examples/overwatch test -- src/brandAssets.test.ts`.

### Nils's codex · May 26, HKT, 2026

**Park UI port completed with SDK browser runtime.** Overwatch now uses Park's Vite/vanilla TypeScript/Tailwind UI shell, stages the generated network/domain/geometry JavaScript/WASM packages, and replaces Park's HTTP/WebSocket data modules with browser-local SDK runtime adapters for cluster state, Discovery listing, participant info, catalogs, registries, and streams. The smoke harness now drives the Park domain modal in two isolated browser contexts, verifies browser peer reachability through Discovery, reads a generated SDK stream frame, and asserts that no app `/api/*` route is called.

Tests: `npm --prefix examples/overwatch run typecheck`; `npm --prefix examples/overwatch test`; `npm --prefix examples/overwatch run build`; `npm --prefix examples/overwatch run smoke`.
