# auki-dsc-web

wasm-bindgen surface for `auki-dsc` — bake + findPath for browser debug (gotu-web PathfindDebug).

```bash
# from auki-sdk root
RUSTFLAGS='--cfg getrandom_backend="wasm_js"' \
  wasm-pack build labs/bindings/web/auki-dsc-web \
  --target web --out-dir pkg --dev

# copy into gotu-web (throwaway):
#   cp -R labs/bindings/web/auki-dsc-web/pkg ../gotu-web/src/lib/auki-dsc-wasm
```

```ts
import init, { WasmNavMesh } from "./auki_dsc_web.js";
await init();
const nav = WasmNavMesh.bake(objText, 0.5);
const path = nav.findPath([start, end]); // { full, totalDistance, waypoints }
```

**Radius:** metres → voxels via ceil. DSC HTTP `radius: 0.05` erodes 0 voxels — use `0.0` for bit-match.
