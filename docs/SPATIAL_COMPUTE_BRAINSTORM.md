# Spatial compute libraries for the SDK — brainstorm

**Status:** Brainstorm (not a decision doc)
**Date:** 2026-09-21
**Owner:** Jason
**Goal:** make [`domain-spatial-computer`](https://github.com/aukilabs/domain-spatial-computer) obsolete by replicating its functionality as SDK library(s).
**Related:** [`auki-geometry`](../crates/auki-geometry), [`auki-maps`](../crates/auki-maps), [`auki-mappers`](../crates/auki-mappers), [`auki-registry` MapBody](../crates/auki-registry), [Component execution](wiki/Component-Execution.md), [robot-runner-kit `auki_navigation`](https://github.com/aukilabs/robot-runner-kit), DMT (`navmesh_v1` / `occlusionmesh_v1`), Gotu / cactus-search / peyote / cactus Unity.

---

## Working notes (capture first)

### DSC is already a library pretending to be a service

`domain-spatial-computer` is a **Deno/Oak HTTP microservice** (port 8000) plus a newer **Python package `dsc`** (`python_local` / `python/`). No Rust. No libp2p. No DDS advertisement. No gRPC. It is a **stateless client of Domain Server REST**: app-key → DDS token → DS `GET .../data?name=navmesh_v1|occlusionmesh_v1&data_type=obj` → bake Recast in-process → return JSON.

That is the wrong shape for the SDK. `auki-geometry` / `auki-maps` are IO-free. Detectors/Mappers are in-process components that *emit* Resources. DSC is a remote function call that re-downloads and re-bakes the same OBJ on every request (Deno has **zero mesh cache**).

The Python port already admitted this: `python/README.md` — “Other apps import and call functions — there is no HTTP server.” Robots followed: [`robot-runner-kit/auki_navigation`](https://github.com/aukilabs/robot-runner-kit) vendors that logic and talks to DS via `DomainClient`, not `localhost:8000`.

**Killing the HTTP process is not the hard part.** Robots already did. The hard part is **one bake+query implementation** that Gotu, peyote, cactus, robots, and any future peer all bind, instead of five.

### Production apps almost never call DSC HTTP

| Caller | Engine | DSC HTTP? |
|--------|--------|-----------|
| **gotu** / **cactus-search** / **gotu-zappar** | `@recast-navigation` in-browser (`threeToSoloNavMesh`) | No |
| **cactus Unity** | `UnityEngine.AI.NavMesh` after OBJ → `NavMeshSurface.BuildNavMesh()` | No |
| **peyote** `features/spatial-ar/guidance/` | custom XZ grid + BFS, `CLEARANCE = 0.05` (Unity Humanoid radius) | No |
| **gotu-web `PathfindDebug`** | fetches DSC | **Yes — debug only**, hardcoded `localhost:8000` + API key |
| **robots** | `auki_navigation` (pynavmesh / trimesh / A* fallback) | No — already local |
| **park / pulse / auki-sdk / cactus-backend** | — | No client found |

Customer docs still list DSC as a Cactus “core service” that “calculates paths and optimized routes.” That is the **product narrative**, not the current client graph. Cactus Smart Routing / staff AR is local Recast or Unity. If some closed mobile build still hits the k8s ingress, it is not in the trees we have.

**Implication:** v0 success is **library parity + consumer cutover**, not “stand up another `/spatial/findpath`.” An HTTP façade is optional compatibility, not the product.

### What DSC actually implements (the replacement surface)

Five endpoints on `develop` (`swagger.yaml` / `src/routes/spatial/`):

| Op | Input | Output | Mesh |
|----|-------|--------|------|
| `POST /spatial/findpath` | `domainId`, `wayPoints[]`, `radius?` | `full`, `segments[]`, distances, snapped `waypoints` (`OnMeshPoint`) | bake `navmesh_v1` |
| `POST /spatial/findoptimizedpath` | + `fixedEnd?` | same + `waypointIndices` | greedy nearest-neighbor, **not TSP** |
| `POST /spatial/restricttonavmesh` | `target`, `radius?` | `{original, restricted, direction}` | snap; if closer than `radius`, push out. **`radius` here is min-separation, not agent radius** — bake forced to `0` |
| `POST /spatial/raycast` | `ray` as Pose / FlatPose / matrix | `hits[]` | `occlusionmesh_v1` via THREE / trimesh |
| `POST /spatial/crosssection` | `height`, `fileType` png\|bmp\|stcm, ppm, colors | image + ROS-style `map.yaml` | slice occlusion at Y=`height` |

Unmerged, already needed by robots: `POST /spatial/findoptimizedsegmentpath` (`feat/optimized-segment-path`). Samples each named OBJ object along its principal axis, then NN-orders reversible corridor segments. Ported as `auki_navigation/segment_path.py`.

Not in DSC (do not pretend it is): occupancy ingest, floor detection, portal graphs, crowd/DetourCrowd, glTF ingest, collision physics, generic AABB/POI queries, writing meshes back to DS (`updateDomainData` exists and is unused).

STCM export shells out to closed `stcm_creater_{amd64,arm64}` ELFs. Linux-only. Do not block v0 on this binary.

### Authoritative assets stay DMT / Domain Server

| Name | Type | Who writes | Who reads |
|------|------|------------|-----------|
| `navmesh_v1` | Wavefront OBJ | DMT (`useNavmesh.ts`) | DSC, Gotu, peyote, robots, cactus Unity |
| `occlusionmesh_v1` | OBJ | DMT (`useOcclusionmesh.ts`) | DSC raycast/cross-section, Gotu occlusion **display**, cactus heatmaps |

Consumers do not generate these. SDK replaces **compute**, not **authoring**. Reconstruction Server is a different pipeline (captures → refined spatial assets).

OBJ group names can encode Unity area types (`guid:areaType`); peyote treats area `0` as walkable, else blocked. Recast JS/Python mostly ignore that and treat the whole mesh as walkable geometry to bake.

### Bake params are already an interoperability bug

Same OBJ, five different “walkable” surfaces:

| Baker | `walkableRadius` / agent | `cs` | `ch` | notes |
|-------|--------------------------|------|------|-------|
| DSC Deno | `radius ?? 0` | 0.05 | **0.01** | re-bake per request |
| Gotu / cactus-search | **0.05** | 0.05 | **0.05** | in-memory Recast |
| DSC Python / robots | `ROBOT_RADIUS` default **0.4**; `0` clamped to `1e-6` (pynavmesh crash) | 0.05 | 0.01 | + flip downward tris; A* fallback if pynavmesh fails |
| peyote grid | `CLEARANCE = 0.05` | 2D cell grid, max 120k cells | — | Unity `# unity` Z-flip; not Recast |
| cactus Unity | Humanoid / `NavMeshSurface` defaults | — | — | different engine entirely |

Gotu-web PathfindDebug even keeps a **hardcoded reference path** because live DSC ≠ client bake. A shared library that does not **lock bake params + agent radius as part of the query contract** just reproduces this.

### SDK already owns a different spatial stack

| Have | Where | Relevance |
|------|-------|-----------|
| Frame conventions + SE(3) | `auki-geometry` | paths must name a frame; do not invent Domain-as-frame |
| Sparse occupancy voxels | `MapBody::Voxel` + `auki-maps` + `auki-mappers` | **not** a navmesh; future *source* for auto-nav, not v0 input |
| Robot collision STL / URDF | Device Model + Blob | kinematics, not walkable floor |
| `convert_pose` over Pose Logs | documented, not shipped | path results should still be points **in a named frame** |

`MapBody` is `Voxel` only. No Mesh / NavMesh / HeightField / Costmap variant. Occupancy ≠ navigable surface. Do not stuff Recast into `auki-maps`.

Five questions: this work is **Spatial** (where can I walk / what occludes), not Networking. It must not grow `AukiPeer`.

### `auki-libs` is the wrong home for the primitive

`auki-libs` is product peers (fleet, inspect, URDF-FK). Path query is closer to `auki-geometry` than to `peer-browser`. A **nav-compute peer** that publishes a baked blob is a later *component/app*, not the library.

---

## Working thesis

**v0 is an IO-free Rust crate (or two) that takes triangle meshes + agent params and answers DSC’s queries.** Bindings: Python (replace `auki_navigation` / `dsc`) and Wasm (replace Gotu Recast JS + peyote grid). Domain fetch, HTTP, and Map Registry stay outside.

DSC-obsolete means:

1. Same ops, same response shapes (so robot-runner / PathfindDebug can swap).
2. One Recast-class baker, one query object, explicit bake profile.
3. No Deno process in the deploy diagram.
4. Consumers import the crate; they already have DS credentials.

It does **not** mean: auto-navmesh from voxels, portal graphs, Unity replacement in cactus native, or a Posemesh “spatial compute protocol.”

---

## Crate split (why “library(s)”)

Mirror `auki-geometry` (math) vs `auki-maps` (map state) vs `auki-mappers` (component). Do not make one kitchen-sink `auki-spatial`.

### A. `auki-nav` (core, v0)

IO-free. No HTTP, no DDS, no `AukiPeer`.

- Ingest: triangle mesh (indexed verts + faces + optional per-group area ids). OBJ parse can live here or a tiny `auki-obj` module — Wavefront is the v0 on-disk format, not a protocol.
- Bake: Recast-style solo navmesh from that mesh + **bake profile** (`cs`, `ch`, `walkableRadius`, `walkableHeight`, region/detail knobs). Profile is data, not globals.
- Query: `find_closest_point`, `compute_path`, `restrict` (snap + optional min-separation).
- Assemble: ordered waypoints → `PathResult` (`full`, `segments`, distances, `OnMeshPoint`s) matching DSC JSON so cutover is mechanical.
- Policy helpers that are still “nav,” not product:
  - greedy NN waypoint order (`find_optimized_path`)
  - named-submesh corridor sampling (`find_optimized_segment_path`)

Pin coordinates as **metres in a caller-supplied frame id** (string or `RegistryRef`). Do not convert conventions inside this crate — call `auki-geometry` first if the mesh is Unity/OpenGL and the agent is `ros_body`.

### B. `auki-occlusion` (v0 or same crate, separate module)

Also IO-free. Different mesh, different algorithm.

- Raycast vs triangle soup / BVH (`occlusionmesh_v1`).
- Horizontal cross-section at Y=`height` → binary occupancy image + ROS `map.yaml` (`origin`, `resolution`, thresh). This is what robots call `DomainNavigation.get_map()`.
- STCM: optional adapter around `stcm_creater` **or drop**. PNG/YAML is the portable bit.

Could be `auki_nav::occlusion` if we want fewer crates. Split if Recast deps must not infect a raycaster-only consumer (heatmap / Gotu visual occluder).

### C. Not a crate — application I/O

| Thing | Owner |
|-------|--------|
| Fetch `navmesh_v1` / `occlusionmesh_v1` from Domain Server | existing DS HTTP / `DomainClient` / future Blob+Registry |
| Disk cache + 300s revalidate | robot-runner `MeshStore` today; stay in the app or a thin helper outside SDK |
| `x-api-key` Oak server | **do not rebuild**. If cactus-backend needs a route API, it calls `auki-nav` in-process |
| DMT authoring / undo / upload | DMT |
| POI / product locations | Gotu PocketBase, cactus DPM — waypoints into `find_path`, not mesh |

### D. Later — Resource, not v0

`MapBody::NavMesh { frame, agent, cell, blob_ref }` (or a sibling registry kind) + baked tiles as Blob v1. Then a Mapper-shaped component: occupancy Map Log and/or authored OBJ → published navmesh tip.

That is how spatial compute becomes a Domain Resource other peers can **consume without baking**. It is also how we stop every client from re-rasterizing Recast. **Do not block DSC-kill on registry schema.** Today the contract is “DS OBJ named `navmesh_v1`.” Honor that first.

Occupancy → navmesh extractor is a **component** (`auki-mappers` cousin), not `auki-nav` itself: voxels in, triangle/navmesh Resource out.

### E. Explicitly not in SDK facade

No Recast inside `AukiPeer`. No `/auki/nav` protocol in v0. Remote “please pathfind” is a product protocol later, if ever — Component-Execution already says remote compute ≠ Resource.

---

## Engine choice — **pure Rust** (locked 2026-09-21)

Need Recast **generation** + a path **query**, compiling to **wasm32-unknown-unknown** (Gotu/peyote) and **linux-aarch64** (robots) with no Deno/Node and no C++ sys crate.

Working stack (not coded yet):

| Role | Crate | Notes |
|------|-------|--------|
| Bake | [`rerecast` 0.4](https://crates.io/crates/rerecast) | Jan Hohenheim Recast port. `TriMesh` → heightfield → compact → contours → `PolygonNavmesh`. glam 0.32. no_std/`libm` available. |
| Query | [`landmass` 0.9](https://crates.io/crates/landmass) | Same orbit as rerecast (andriyDev). A* + funnel on a poly mesh. **Not Detour.** Z-up internally; public API stays Recast/OpenGL Y-up and converts at the boundary. |

Looked at, rejected for v0:

| Crate | Why not |
|-------|---------|
| `landmark` + `waymark` (wowemulation-dev recast-rs) | Actual Recast+Detour pair, wasm OK, but **MSRV 1.92**. SDK is 1.89 / rustc here is 1.90. |
| crates.io `recast` / `detour` | Name-squatted (text rewriter / function hooks). Degenerate-Laboratories recast-rs never published under those names. |
| `recastnavigation-sys` | C++. Wasm + aarch64 robot freeze is the pain we are avoiding. |
| `vleue_navigator` / `polyanya` | Bevy-shaped; 2D any-angle. Fine for peyote floors, wrong as the DSC/Gotu Recast replacement. |

**Still unproven (the actual spike):** bake+path golden vs DSC Deno on a real `navmesh_v1`. If Hausdorff is ugly, fall back to C++ native + Rust wasm with a documented epsilon — do not pre-empt that.

Python: PyO3 sibling `auki-nav-py` (same pattern as `auki-geometry-py`). Delete vendored `dsc/` from robot-runner once that’s real.

Wasm: either a dedicated `auki-nav-web` or fold path query into `auki-sdk-web`. Prefer **dedicated** — Recast is heavy and protocol WASM should not pay for it at startup.

Swift/Unity: not v0. Cactus native keeps `NavMeshSurface` until someone cares.

Python: PyO3 sibling `auki-nav-py` (same pattern as `auki-geometry-py`). Delete vendored `dsc/` from robot-runner once that’s real.

Wasm: either a dedicated `auki-nav-web` or fold path query into `auki-sdk-web`. Prefer **dedicated** — Recast is heavy and protocol WASM should not pay for it at startup.

Swift/Unity: not v0. Cactus native keeps `NavMeshSurface` until someone cares.

---

## Wire / type contract to keep (cutover)

Copy DSC shapes, then stop adding Deno-only fields.

```text
Vector3           { x, y, z }          // metres, caller frame
OnMeshPoint       { original, adjusted, isOffMesh }
PathSegment       { points, hasOffMeshStart, hasOffMeshEnd }
PathResult        { full, segments, totalDistance, segmentsDistances }
RestrictResult    { original, restricted, direction }
RayHit            whatever raycast.ts actually returns today
CrossSection      { image_bytes, map_yaml }
BakeProfile       { cs, ch, walkable_radius, walkable_height, ... }  // NEW, required
```

`domainId` is **not** a nav input. It is fetch context. Library API:

```text
NavMesh::bake(mesh, profile) -> NavMesh
NavMesh::find_path(waypoints) -> PathResult
NavMesh::restrict(point, min_separation) -> RestrictResult
OcclusionMesh::raycast(origin, dir) -> [Hit]
OcclusionMesh::cross_section(height, ppm) -> (image, yaml)
```

Agent radius belongs on **bake** (walkable radius) and optionally as a query filter — not overloaded onto `restrict.radius`.

---

## Consumer cutover order (how DSC actually dies)

1. **Spike `auki-nav` + golden tests** vs DSC Deno on one real domain OBJ (path, optimized, restrict).
2. **Python bind → robot-runner-kit** `DomainNavigation` swaps `dsc` for `auki-nav-py`. This is the only production local port that already matches DSC ops (incl. segment path + cross-section).
3. **Wasm bind → gotu-web PathfindDebug** (kills the last HTTP client we can see). Then Gotu / cactus-search `NavigationProvider` (delete `threeToSoloNavMesh`).
4. **peyote guidance** — replace the 2D grid. Watch Unity Z-flip (`# unity` in OBJ) — that is a **mesh convention** bug, handle at ingest with `auki-geometry`, not inside A*.
5. **cactus Unity** — last; different engine. Either keep Unity bake or call a native staticlib. Not required to undeploy DSC.
6. **Helm chart / k8s DSC** — undeploy when (2)+(3) are live and customer-docs drop the service. Until then the process can sit idle.

Cactus-backend does not need a new route API unless some server-side Smart Routing still exists outside the app. If it does, in-process `auki-nav` in that service — still not a Domain peer.

---

## What would make this worse

- **Reimplement Deno 1:1 in Axum.** Same microservice, new language. Robots already rejected that.
- **Bake inside the crate with hidden defaults.** Lock a named profile (`humanoid_v0`, `robot_r40_v0`) or every app will drift again.
- **Put DS HTTP in `auki-nav`.** Then the crate cannot run in peyote against a mesh already in memory, and tests need DDS.
- **Treat occupancy maps as navmeshes.** Different `MapBody`. Bridge later via a Mapper.
- **Portal-aware multi-floor graphs in v0.** Gotu portals are localization / domain entry, not path waypoints. DSC has none of this.
- **Crowd / dynamic obstacles in v0.** DetourCrowd is a sequel.
- **STCM as a success metric.** PNG + YAML unblocks SLAM-style maps; the ELF is a proprietary encoder.

---

## Open questions

1. **One crate or two?** `auki-nav` + occlusion module vs `auki-nav` + `auki-occlusion`. Recast dependency weight vs heatmap-only consumers.
2. **Engine locked: rerecast bake + landmass query.** Open: does it match DSC Deno on a real store OBJ, or do we eat an epsilon / fall back to C++ native?
3. **Bake profile ownership:** hardcoded SDK presets vs mesh-sidecar JSON next to `navmesh_v1` vs later `MapBody` fields. v0 at least: explicit struct, no `radius ?? 0`.
4. **Frame:** DMT OBJs are which convention? Peyote special-cases Unity Z. If authored in OpenGL Y-up, Gotu camera paths are already in that frame — document it.
5. **Is any production client still hitting DSC ingress?** Customer self-host diagrams say yes. Need a one-line confirm from whoever owns Cactus mobile Smart Routing. If yes, cactus-backend-in-process is the compatibility shim, not a new service.
6. **Should baked navmesh become a Blob?** Yes eventually. v0 can keep “bake every load” (Gotu already does) as long as bake is fast and cached in-process (Python `MeshStore` is the model; Deno is the anti-model).
7. **Segment path on `develop`?** Promote into the library even if DSC never merges the route — robots already depend on it.

---

## Proposed first spike (when we leave brainstorm)

Not implementation yet — this is the smallest thing that de-risks the thesis:

1. Check a real `navmesh_v1` into fixtures (or fetch once and freeze).
2. Capture DSC Deno `findpath` / `restrict` / `findoptimizedpath` JSON as golden.
3. Sketch `auki-nav` API + one Recast backend.
4. Diff paths (Hausdorff / waypoint count / off-mesh flags).
5. Report: engine choice, bake-profile defaults, crate split (1 vs 2).

Out of spike: PyO3, Wasm, peyote cutover, MapBody, HTTP.

---

## Locked decisions (v0)

| Decision | Lock |
|----------|------|
| **DSC obsolete** | Same results locally (ops + shapes); not HTTP / not remote compute. |
| **Crate** | Split: `auki-geometry` (ingest + raycast), `auki-raster` (Y-slice), `auki-navigation` (bake + path). |
| **Engine** | `rerecast` 0.4 bake + `landmass` 0.9 query. No Detour. `landmass_rerecast` skipped (Bevy). Manual Y-up↔Z-up at boundary. |
| **Bake API** | `BakeProfile` with presets `gotu()`, `dsc(r)`, `robot_r40()`, `restrict_bake()`. Radius = walkable erosion. Restrict min-separation is query-time. |
| **Ops** | path, optimized (greedy NN), restrict, raycast, cross-section (PNG+YAML, no STCM), segment path. |
| **Cutover** | Spike + golden measurement → robot-runner-kit Python → gotu-web PathfindDebug → Helm undeploy. |
| **Golden** | Hausdorff / max delta measurement vs DSC Deno; soft epsilon (0.5 m). Not a bit-identical merge gate. |

v0 shipped on `feature/dsc` as `labs/auki-dsc`; renamed/split to `auki-navigation` + `auki-geometry` + `auki-raster`.

---

## File map (investigation)

| Path | Why |
|------|-----|
| `domain-spatial-computer/src/routes/spatial/*.ts` | HTTP ops |
| `domain-spatial-computer/src/utils/recast/navmesh.ts` | Deno bake params |
| `domain-spatial-computer/python/src/dsc/` | library port |
| `domain-spatial-computer/swagger.yaml` | wire types |
| `robot-runner-kit/auki_navigation/` | production local consumer + segment path |
| `gotu/gotu/contexts/NavigationProvider.tsx` | client Recast |
| `peyote/features/spatial-ar/guidance/navigation-{mesh,route}.ts` | 2D grid fork |
| `gotu-web/src/compositions/threed/PathfindDebug.tsx` | last HTTP client |
| `cactus/.../NavMeshSystem.cs` | Unity bake |
| `auki-sdk/crates/auki-registry` `MapBody` | occupancy only |
| `auki-sdk/crates/auki-geometry` | pattern for IO-free spatial crate |
| `AukiCustomerDocumentation/.../components-and-interactions.mdx` | DSC as Cactus routing service (narrative) |
