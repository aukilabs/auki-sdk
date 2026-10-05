# auki-navigation-web

wasm-bindgen for bake + `findPath`. Mesh ingest (`parseObj` / `fromIndexed`) is `auki-geometry` under the hood.

```ts
const mesh = WasmTriangleMesh.parseObj(objText); // or .fromIndexed(verts, tris)
const nav = WasmNavMesh.bake(mesh, 0.05);
const path = nav.findPath([start, end]);
```

`bakeObj` is a debug shortcut.
