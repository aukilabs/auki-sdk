// Point-cloud tile — Dagaz #5. Subscribes to the libp2p pointcloud
// stream cache via `subscribePointCloud({ peer_id, sensor_id })`,
// decodes the CDR-encoded `PointCloud2` payload to interleaved xyz
// `Float32Array`, and renders it as `THREE.Points`. OrbitControls
// (three.js examples) lets the operator look around.
//
// Render-on-demand — no animation loop. New frames call
// `requestRender()`; OrbitControls' `change` event re-renders on
// pointer / scroll input.

import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { SensorEntry } from "../../../data/registry";
import { iconInfo } from "../../../icons";
import {
  openInspector,
  type InspectorContent,
  type InspectorHandle,
} from "../../../shell/inspectorDrawer";
import { makeChromeBtn, makeTileChrome } from "./chrome";
import type { TileHandle } from "../tile";
import { type PointCloudPreviewFrame } from "../../../data/pointcloudPreview";
import { subscribePointCloudSynced } from "../../../data/peerSync";
import {
  fetchFrameConventionMatrix,
  fetchStreamDescriptor,
} from "../../../data/registry";
import { shortName } from "./names";
import { makeRecordingInspectorControl } from "../recordingControl";

/** Maximum points the GPU buffer holds. The K1's StereoNetNode produces
 * ~46K points/frame raw; producer-side voxel decimation (Dagaz D3) can
 * cut that further. 100K gives headroom without bloating GPU memory
 * (1.2 MB for the position buffer). */
const MAX_POINTS = 100_000;
const DEFAULT_FOV_DEG = 60;

// ROS/OpenCV optical camera coordinates (+X right, +Y down, +Z
// forward) -> Three.js camera/world coordinates (+X right, +Y up,
// -Z forward). Used only as a compatibility fallback when an organized
// cloud gives us camera intrinsics before the producer's Frame Registry
// descriptor is available.
const OPTICAL_TO_THREE_CAMERA = new THREE.Matrix4().set(
  1, 0, 0, 0,
  0, -1, 0, 0,
  0, 0, -1, 0,
  0, 0, 0, 1,
);

type InferredProjection = NonNullable<PointCloudPreviewFrame["projection"]>;
type RenderViewport = { x: number; y: number; width: number; height: number };

export function makePointCloudTile(
  spec: {
    sensor_id: string;
    daemon_url: string;
    peer_id: string;
    entry?: SensorEntry;
  },
  opts: { onClose: () => void },
): TileHandle & { requestRender(): void } {
  const chrome = makeTileChrome({
    sensor_id: spec.sensor_id,
    entry: spec.entry,
    onClose: opts.onClose,
  });

  // ─── body: Three.js canvas + "no frames" overlay ──────────────────
  const canvas = document.createElement("canvas");
  canvas.className = "absolute inset-0 w-full h-full block";
  chrome.body.appendChild(canvas);

  const overlay = document.createElement("div");
  overlay.className =
    "absolute inset-0 flex flex-col items-center justify-center gap-1 text-rule pointer-events-none";
  overlay.innerHTML = `
    <span class="text-[11px] uppercase tracking-[0.2em] text-paper/70">no frames</span>
    <span class="text-[11px] text-rule/70">awaiting first point-cloud frame</span>
  `;
  chrome.body.appendChild(overlay);

  // ─── bottom info ──────────────────────────────────────────────────
  chrome.bottomInfo.innerHTML = `
    <span class="truncate" title="${spec.sensor_id}">${spec.sensor_id}</span>
    <span class="text-rule/70 shrink-0" data-region="status">awaiting frames</span>
  `;
  const statusEl = chrome.bottomInfo.querySelector(
    '[data-region="status"]',
  ) as HTMLElement;

  const inspectBtn = makeChromeBtn(iconInfo(13), "Inspect sensor");
  chrome.bottomActions.append(inspectBtn);

  // ─── Three.js set-up ───────────────────────────────────────────────
  const renderer = new THREE.WebGLRenderer({
    canvas,
    antialias: true,
    alpha: true,
  });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  renderer.setClearColor(0x0f0f0f, 1);
  renderer.autoClear = false;

  const scene = new THREE.Scene();
  // Camera default: a conservative free-view orbit. If the incoming
  // PointCloud2 is organized and its xyz samples fit a pinhole model,
  // the tile switches to a source-camera projection inferred from the
  // cloud itself. That path gives Realman / RGB-D-style producers their
  // actual FOV without waiting for a separate calibration registry.
  const camera = new THREE.PerspectiveCamera(DEFAULT_FOV_DEG, 1, 0.05, 100);
  camera.up.set(0, 1, 0);
  camera.position.set(0, 0, 1.5);
  camera.lookAt(0, 0, 0);

  // Pre-allocate the position buffer once; every incoming frame copies
  // its decoded xyz into this same array and bumps `needsUpdate`. This
  // keeps GPU buffer reuse stable — `THREE.BufferAttribute.setUsage`
  // tells the WebGL driver to expect frequent updates.
  const positionArray = new Float32Array(MAX_POINTS * 3);
  const positionAttr = new THREE.BufferAttribute(positionArray, 3);
  positionAttr.setUsage(THREE.DynamicDrawUsage);
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", positionAttr);
  geometry.setDrawRange(0, 0);

  const material = new THREE.PointsMaterial({
    size: 0.015,
    sizeAttenuation: true,
    color: 0xffffff,
  });
  const points = new THREE.Points(geometry, material);
  // Producer-frame → Three.js world transform. We start with identity
  // and replace it asynchronously once Park can resolve the producer's
  // frame metadata:
  //
  //   1. fetch `/api/streams/<peer>/<sensor>/descriptor` → producer's
  //      `(frame_id, frame_hash)` from the StreamDescriptor (PR #130).
  //   2. ask Park Rust for
  //      `/api/daemons/<url>/registries/frames/<id>/<hash>/convention_matrix`.
  //      That endpoint calls `auki_geometry::convention_matrix` against
  //      `FrameRegistryEntry::opengl("park/three_js_world")`, keeping
  //      SDK convention semantics out of this browser bundle.
  //   3. set `points.matrix` and re-render.
  //
  // Identity-until-resolved means a brand-new tile renders in the
  // producer's native axes for the first ~100 ms — at worst the cloud
  // appears rotated until the operator orbits, never invisible.
  // Same fallback if either fetch fails (legacy producers without
  // FrameRegistryEntry exposure, peer-only producers before the SDK's
  // `/auki/registries/0.0.1` libp2p exchange lands, etc.).
  //
  // Three.js Matrix4.set is row-major.
  points.matrixAutoUpdate = false;
  points.matrix.identity();
  points.matrixWorldNeedsUpdate = true;
  scene.add(points);

  const resolvedConventionMatrix = new THREE.Matrix4();
  let hasResolvedConventionMatrix = false;
  let sourceProjection: InferredProjection | null = null;
  let sourceProjectionActive = false;
  let renderSize = { width: 1, height: 1 };
  let renderViewport: RenderViewport | null = null;

  const refreshPointMatrix = () => {
    if (hasResolvedConventionMatrix) {
      points.matrix.copy(resolvedConventionMatrix);
    } else if (sourceProjectionActive) {
      points.matrix.copy(OPTICAL_TO_THREE_CAMERA);
    } else {
      points.matrix.identity();
    }
    points.matrixWorldNeedsUpdate = true;
  };

  const refreshCameraProjection = () => {
    if (sourceProjection) {
      renderViewport = containViewport(
        renderSize.width,
        renderSize.height,
        sourceProjection.width / sourceProjection.height,
      );
      applyPinholeProjection(camera, sourceProjection);
      return;
    }
    renderViewport = null;
    camera.fov = DEFAULT_FOV_DEG;
    camera.aspect = renderSize.width / renderSize.height;
    camera.updateProjectionMatrix();
  };

  // ─── render-on-demand ─────────────────────────────────────────────
  let unloaded = false;
  let inspector: InspectorHandle | null = null;
  let lastFrame: PointCloudPreviewFrame | null = null;
  const recordingControl = makeRecordingInspectorControl({
    peerId: spec.peer_id,
    sensorId: spec.sensor_id,
    onChange: () => {
      if (inspector?.isOpen()) inspector.update(buildInspectorContent());
    },
  });
  const requestRender = () => {
    if (unloaded) return;
    renderer.setScissorTest(false);
    renderer.setViewport(0, 0, renderSize.width, renderSize.height);
    renderer.clear(true, true, true);
    if (renderViewport) {
      renderer.setViewport(
        renderViewport.x,
        renderViewport.y,
        renderViewport.width,
        renderViewport.height,
      );
      renderer.setScissor(
        renderViewport.x,
        renderViewport.y,
        renderViewport.width,
        renderViewport.height,
      );
      renderer.setScissorTest(true);
    }
    renderer.render(scene, camera);
    renderer.setScissorTest(false);
  };

  // Async resolve the producer's frame and apply the right transform.
  // Polls the descriptor endpoint until it returns a non-null body
  // (the StreamDescriptor only exists once the SDK's Accept lands,
  // which can race with tile mount). Once resolved, fetches Park's
  // server-computed convention matrix and applies it exactly once.
  //
  // Declared AFTER `requestRender` because the void-call evaluates its
  // arguments synchronously — moving it above triggered a TDZ
  // ReferenceError on `requestRender`, which propagated up through
  // `stage.setTiles` into the `subscribeSensorLogs` listener, hit
  // sensorLogs.ts's outer try/catch, and silently fired every listener
  // with `state=null`. The visible symptom was "inspector view's
  // sensor strip flashes 4 chips then clears every poll cycle."
  let frameResolveCancelled = false;
  void resolveAndApplyFrame(
    spec,
    (matrix) => {
      hasResolvedConventionMatrix = true;
      resolvedConventionMatrix.copy(matrix);
      refreshPointMatrix();
      requestRender();
    },
    () => frameResolveCancelled,
  );

  const controls = new OrbitControls(camera, canvas);
  controls.enableDamping = true;
  controls.dampingFactor = 0.1;
  // Orbit around the body-frame origin — matches `camera.lookAt(0,0,0)`
  // above so the starting view is unambiguous.
  controls.target.set(0, 0, 0);
  // Sync OrbitControls' internal spherical state with the camera's
  // current up vector + position + target. Without this update,
  // OrbitControls can compute its first orbital axis from a stale
  // up vector and produce a tilted starting view.
  controls.update();
  // OrbitControls' `change` event fires on every interaction frame —
  // wiring it to requestRender preserves render-on-demand semantics
  // without an `animate()` loop.
  controls.addEventListener("change", requestRender);

  const activateSourceProjection = (projection: InferredProjection) => {
    if (sourceProjection && sameProjection(sourceProjection, projection)) return;
    const firstProjection = sourceProjection === null;
    sourceProjection = projection;
    if (firstProjection) {
      sourceProjectionActive = true;
      camera.position.set(0, 0, 0);
      camera.up.set(0, 1, 0);
      camera.lookAt(0, 0, -1);
      controls.target.set(0, 0, -1);
      refreshPointMatrix();
    }
    refreshCameraProjection();
    if (firstProjection) controls.update();
  };

  const resize = () => {
    if (unloaded) return;
    const rect = chrome.el.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;
    renderSize = {
      width: Math.max(1, Math.floor(rect.width)),
      height: Math.max(1, Math.floor(rect.height)),
    };
    renderer.setSize(renderSize.width, renderSize.height, false);
    refreshCameraProjection();
    requestRender();
  };
  const ro = new ResizeObserver(resize);
  ro.observe(chrome.el);
  resize();

  // ─── stream subscription ──────────────────────────────────────────
  let firstFrame = true;
  let lastFrameAt = 0;
  let fpsTimer: number | null = null;
  let framesSinceTick = 0;
  const updateStatus = (label: string) => {
    if (statusEl) statusEl.textContent = label;
    if (inspector?.isOpen()) inspector.update(buildInspectorContent());
  };
  const onFrame = (frame: PointCloudPreviewFrame | null) => {
    if (unloaded || !frame) return;
    lastFrame = frame;
    const triplets = Math.min(frame.pointCount, MAX_POINTS);
    const writeLen = triplets * 3;
    positionArray.set(frame.positions.subarray(0, writeLen));
    // Zero out any tail from a previous larger frame so stale points
    // don't ghost when the cloud shrinks.
    if (writeLen < positionArray.length) {
      positionArray.fill(0, writeLen);
    }
    positionAttr.needsUpdate = true;
    geometry.setDrawRange(0, triplets);
    geometry.computeBoundingSphere();
    if (frame.projection) activateSourceProjection(frame.projection);
    if (firstFrame) {
      firstFrame = false;
      overlay.style.display = "none";
    }
    framesSinceTick += 1;
    lastFrameAt = frame.receivedAt;
    requestRender();
    if (inspector?.isOpen()) inspector.update(buildInspectorContent());
  };
  fpsTimer = window.setInterval(() => {
    if (framesSinceTick === 0) {
      if (!firstFrame) updateStatus("stalled");
      return;
    }
    const fps = framesSinceTick;
    framesSinceTick = 0;
    updateStatus(`${fps.toFixed(0)} fps · ${Math.round(performance.now() - lastFrameAt)}ms ago`);
  }, 1000);
  const unsubscribe = subscribePointCloudSynced(
    { peer_id: spec.peer_id, sensor_id: spec.sensor_id },
    onFrame,
  );

  function buildInspectorContent(): InspectorContent {
    return {
      title: shortName(spec.sensor_id),
      subtitle: spec.daemon_url,
      badge: {
        label: firstFrame ? "WAITING" : "LIVE",
        tone: firstFrame ? "muted" : "live",
      },
      actions: recordingControl.actions(),
      sections: [
        recordingControl.section(),
        {
          title: "Sensor",
          rows: [
            { key: "sensor_id", value: spec.sensor_id },
            { key: "type", value: "point_cloud", mono: false },
            { key: "peer_id", value: spec.peer_id },
          ],
        },
        {
          title: "Frame",
          rows: lastFrame
            ? [
                { key: "seq", value: String(lastFrame.seq) },
                { key: "points", value: String(lastFrame.pointCount) },
                {
                  key: "shape",
                  value: `${lastFrame.width}x${lastFrame.height}`,
                },
                {
                  key: "frame_id",
                  value: lastFrame.frameId || "-",
                  dim: !lastFrame.frameId,
                },
                {
                  key: "timestamp_ns",
                  value: String(lastFrame.timestamp_ns || "-"),
                  dim: !lastFrame.timestamp_ns,
                },
              ]
            : [{ key: "status", value: "no frame yet", dim: true, mono: false }],
        },
      ],
    };
  }

  function openTileInspector() {
    inspector = openInspector(buildInspectorContent());
  }
  inspectBtn.addEventListener("click", () => openTileInspector());

  return {
    el: chrome.el,
    requestRender,
    dispose() {
      unloaded = true;
      frameResolveCancelled = true;
      ro.disconnect();
      unsubscribe();
      recordingControl.dispose();
      inspector?.close();
      inspector = null;
      if (fpsTimer !== null) clearInterval(fpsTimer);
      controls.dispose();
      geometry.dispose();
      material.dispose();
      scene.remove(points);
      renderer.dispose();
      // Three.js's `dispose()` releases JS-side resources (programs,
      // textures, buffers) but does NOT release the underlying WebGL
      // GPU context. Chrome caps WebGL contexts at 16 per process —
      // once exceeded, contexts get discarded silently and the tab
      // eventually crashes. `forceContextLoss()` invokes the
      // `WEBGL_lose_context` extension to release the GPU side too.
      renderer.forceContextLoss();
    },
    toggleFreeze() {
      // Future: pause the subscription callback's geometry updates so
      // the operator can rotate/zoom a static frame. Today the
      // OrbitControls already let them inspect from any angle on a
      // running cloud.
    },
    snapshot() {
      // Future: read pixels from the canvas and download as PNG.
    },
    close() {
      opts.onClose();
    },
    isFrozen() {
      return false;
    },
    sensorId() {
      return spec.sensor_id;
    },
    setSensorLogs: () => {},
  };
}

/// Resolve the producer's coordinate convention asynchronously and
/// apply the matching `auki-geometry` transform to the tile's points
/// object.
///
/// Polls Park's `/api/streams/<peer>/<sensor>/descriptor` until it
/// returns the StreamDescriptor (the SDK's Accept can race with tile
/// mount — descriptor isn't recorded until the first substream
/// handshake completes). Caps total wait at ~5s — past that we leave
/// the matrix at identity and the operator falls back to orbit.
///
/// When `frame_id`/`frame_hash` are empty (non-spatial sensors or
/// pre-Frame-Registry producers), this short-circuits to identity.
/// Same for any fetch failure — the cloud just renders in producer-
/// native axes.
async function resolveAndApplyFrame(
  spec: { peer_id: string; sensor_id: string; daemon_url: string },
  onResolve: (matrix: THREE.Matrix4) => void,
  isCancelled: () => boolean,
): Promise<void> {
  const POLL_INTERVAL_MS = 250;
  const MAX_POLLS = 20;
  let descriptor = null;
  for (let i = 0; i < MAX_POLLS; i++) {
    if (isCancelled()) return;
    descriptor = await fetchStreamDescriptor(spec.peer_id, spec.sensor_id);
    if (descriptor) break;
    await new Promise<void>((r) => setTimeout(r, POLL_INTERVAL_MS));
  }
  if (isCancelled() || !descriptor) return;
  if (!descriptor.frame_id || !descriptor.frame_hash) {
    // Producer didn't declare a frame — render in native axes.
    return;
  }
  const m = await fetchFrameConventionMatrix(
    spec.daemon_url,
    descriptor.frame_id,
    descriptor.frame_hash,
  );
  if (isCancelled() || !m) return;
  // Three.js Matrix4.set is row-major; Park's `auki-geometry` endpoint
  // returns row-major. Spread row-by-row.
  const matrix = new THREE.Matrix4().set(
    m[0][0], m[0][1], m[0][2], m[0][3],
    m[1][0], m[1][1], m[1][2], m[1][3],
    m[2][0], m[2][1], m[2][2], m[2][3],
    m[3][0], m[3][1], m[3][2], m[3][3],
  );
  onResolve(matrix);
}

function applyPinholeProjection(
  camera: THREE.PerspectiveCamera,
  projection: InferredProjection,
): void {
  const near = camera.near;
  const far = camera.far;
  const left = (-projection.cx * near) / projection.fx;
  const right = ((projection.width - projection.cx) * near) / projection.fx;
  const top = (projection.cy * near) / projection.fy;
  const bottom = (-(projection.height - projection.cy) * near) / projection.fy;
  camera.aspect = projection.width / projection.height;
  camera.fov = THREE.MathUtils.radToDeg(
    2 * Math.atan(projection.height / (2 * projection.fy)),
  );
  camera.projectionMatrix.makePerspective(
    left,
    right,
    top,
    bottom,
    near,
    far,
    camera.coordinateSystem,
  );
  camera.projectionMatrixInverse.copy(camera.projectionMatrix).invert();
}

function containViewport(
  canvasWidth: number,
  canvasHeight: number,
  targetAspect: number,
): RenderViewport {
  const canvasAspect = canvasWidth / canvasHeight;
  if (canvasAspect > targetAspect) {
    const width = Math.max(1, Math.floor(canvasHeight * targetAspect));
    return {
      x: Math.floor((canvasWidth - width) / 2),
      y: 0,
      width,
      height: canvasHeight,
    };
  }
  const height = Math.max(1, Math.floor(canvasWidth / targetAspect));
  return {
    x: 0,
    y: Math.floor((canvasHeight - height) / 2),
    width: canvasWidth,
    height,
  };
}

function sameProjection(a: InferredProjection, b: InferredProjection): boolean {
  return (
    a.width === b.width &&
    a.height === b.height &&
    Math.abs(a.fx - b.fx) < 1e-3 &&
    Math.abs(a.fy - b.fy) < 1e-3 &&
    Math.abs(a.cx - b.cx) < 1e-3 &&
    Math.abs(a.cy - b.cy) < 1e-3
  );
}
