// Robot detail view — composes the per-daemon sidebar (file structure
// tree), the multi-tile stage, and the bottom sensor strip. Live
// streaming only: past-session playback was removed once the live
// cluster demo became Park's only surface.
//
// Mental model:
//   sidebar  = identity header + live file structure tree
//   stage    = active tiles for sensors toggled on in the strip
//   strip    = sensor toggles for the daemon's live session
//
// Keyboard shortcuts:
//   1-9   toggle sensor at that index in the bottom strip
//   F     freeze / unfreeze the focused tile
//   S     snapshot the focused tile
//   C     close (toggle off) the focused tile
//   ?     show shortcut cheatsheet

import type { Daemon } from "../../data/daemons";
import type { DaemonSensorLogs } from "../../data/sensorLogs";
import { subscribeSensorLogs } from "../../data/sensorLogs";
import { subscribeInfo } from "../../data/info";
import { makeSidebar } from "./sidebar";
import { makeStage } from "./stage";
import { makeSensorStrip, type SensorRow } from "./sensorStrip";
import { showCheatsheet } from "../../shell/cheatsheet";
import type { TileSpec } from "./tile";
import {
  fetchCatalog,
  sensorEntryFromCatalogEntry,
  synthEntryFromCatalogKind,
  type SensorCatalogEntry,
  type SensorCatalogKind,
} from "../../data/registry";
import { setAudioListenTarget } from "../../data/inspect";
import { getToggled, setToggled } from "../../data/toggledSensors";

type View = { el: HTMLElement; dispose: () => void };

const SHORTCUTS = [
  { key: "1–9", description: "Toggle sensor at index", group: "Sensors" },
  { key: "F", description: "Freeze focused tile", group: "Tile" },
  { key: "S", description: "Snapshot focused tile", group: "Tile" },
  { key: "C", description: "Close focused tile", group: "Tile" },
  { key: "?", description: "Show this cheatsheet", group: "Help" },
  { key: "Esc", description: "Close any open overlay", group: "Help" },
];

const WORLD_SENSOR_ID = "world";

export function robot(daemon: Daemon | undefined): View {
  const el = document.createElement("main");
  el.className =
    "flex-1 grid grid-cols-[260px_1fr] grid-rows-[1fr_auto] min-h-0";

  const sidebar = makeSidebar(daemon);
  const stage = makeStage(daemon, {
    onCloseTile: (sensor_id) => toggleSensor(sensor_id),
  });
  const strip = makeSensorStrip();

  el.appendChild(sidebar.el);
  el.appendChild(stage.el);
  el.appendChild(strip.el);

  let lastState: DaemonSensorLogs | null = null;
  let realSensors: SensorRow[] = [];
  let liveSensors: SensorRow[] = [];
  let toggled: Set<string> = daemon
    ? (getToggled(daemon.url) ?? new Set<string>())
    : new Set<string>();
  let defaultToggleApplied = toggled.size > 0;
  // Producer's libp2p PeerId — sourced from the daemon's `/api/info`.
  // Null until the first /api/info response lands. Required for live
  // video tiles to subscribe to grimsby streams via
  // `/api/streams/<peer_id>/<sensor_id>/latest.jpg`. When unknown,
  // video tiles aren't pushed to the stage (per pushTilesToStage's
  // gate) so the operator sees the strip + sidebar but no preview
  // until peer_id resolves.
  let peerId: string | null = null;

  // Per-daemon `(sensor_id, sensor_hash) → kind` map sourced from
  // the SDK `/auki/sensors/0.0.1` catalog. Tile routing reads
  // `kind` to dispatch to the right tile component (camera →
  // video, point_cloud → point_cloud, etc.) — substring-guessing
  // from sensor_id mis-routed bracketbot's depth_camera (a point
  // cloud whose id contains "cam") as RGB video.
  //
  // Catalog protocol gives the light one-row-per-sensor routing
  // surface. Registry exchange remains the detail path for geometry
  // and body-specific metadata.
  const sensorKinds = new Map<string, SensorCatalogKind>();
  const sensorCatalog = new Map<string, SensorCatalogEntry>();
  let catalogInFlight = false;
  const kindKey = (id: string, hash: string) => `${id}::${hash}`;

  const persistToggled = () => {
    if (daemon) setToggled(daemon.url, toggled);
  };

  const pushTilesToStage = () => {
    if (!daemon || peerId === null) {
      stage.setTiles([]);
      return;
    }
    const specs: TileSpec[] = [];
    for (const s of liveSensors) {
      if (!toggled.has(s.sensor_id)) continue;
      if (s.sensor_id === WORLD_SENSOR_ID) {
        const pair = worldSensorPair(realSensors);
        if (!pair) continue;
        specs.push({
          kind: "world",
          sensor_id: WORLD_SENSOR_ID,
          daemon_url: daemon.url,
          peer_id: peerId,
          point_cloud_sensor_id: pair.pointCloud.sensor_id,
          joint_sensor_id: pair.joint.sensor_id,
        });
        continue;
      }
      // K1 articulated-pose stream — sawslin Lane D step 5c.
      // Accept both the current registry kind and the conventional
      // `<node_id>/joint_encoders` suffix used by existing rows.
      if (isJointEncodersRow(s)) {
        specs.push({
          kind: "k1_pose",
          sensor_id: s.sensor_id,
          daemon_url: daemon.url,
          peer_id: peerId,
        });
        continue;
      }
      // Route on the SDK-declared sensor type from the registry.
      // Sensors without a resolved registry entry yet are held off the
      // stage until the fetch resolves — same pattern as the peerId
      // gate above, and only ever a brief flash on first open.
      if (!s.entry) continue;
      switch (s.entry.type) {
        case "camera":
          specs.push({
            kind: "video",
            sensor_id: s.sensor_id,
            sensor_hash: s.sensor_hash,
            daemon_url: daemon.url,
            peer_id: peerId,
            entry: s.entry,
          });
          break;
        case "point_cloud":
          specs.push({
            kind: "point_cloud",
            sensor_id: s.sensor_id,
            daemon_url: daemon.url,
            peer_id: peerId,
            entry: s.entry,
          });
          break;
        case "audio":
          // No visual tile for audio sensors — the Dialogue audio
          // consumer pumps them straight to the OS speaker.
          break;
        case "joint_encoders":
          specs.push({
            kind: "k1_pose",
            sensor_id: s.sensor_id,
            daemon_url: daemon.url,
            peer_id: peerId,
          });
          break;
      }
    }
    stage.setTiles(specs);
    pushStateIntoTiles();
  };

  const pushStateIntoTiles = () => {
    const logs = lastState?.sensor_logs ?? [];
    for (const tile of stage.getTiles()) {
      tile.setSensorLogs(logs);
    }
  };

  // Resolve sensor types for any rows missing them. The catalog
  // gives kinds for every sensor on the peer in one round trip; we
  // synthesize a minimal `SensorEntry` from each kind so existing
  // call sites (icon helpers, chrome chip) keep working unchanged.
  // Skips `/joint_encoders` rows — those route by sensor_id suffix
  // and don't have a registry/catalog entry kind. Returns whether
  // anything new was attached so callers can avoid a redundant
  // re-render.
  const attachCachedKinds = (rows: SensorRow[]): boolean => {
    let changed = false;
    for (const r of rows) {
      if (r.entry) continue;
      if (isJointEncodersRow(r)) continue;
      if (!r.sensor_hash) continue;
      const kind = sensorKinds.get(kindKey(r.sensor_id, r.sensor_hash));
      if (!kind) continue;
      const catalogEntry = sensorCatalog.get(kindKey(r.sensor_id, r.sensor_hash));
      const entry = catalogEntry
        ? sensorEntryFromCatalogEntry(catalogEntry)
        : synthEntryFromCatalogKind(r.sensor_id, kind);
      if (entry) {
        r.entry = entry;
        changed = true;
      }
    }
    return changed;
  };

  /// Fetch the per-peer SDK catalog, populate `sensorKinds`, then
  /// re-attach to the live row list and re-render. Single in-flight
  /// fetch at a time — the registry.ts cache layer already TTL-caches
  /// the response so calling this repeatedly is cheap.
  const refreshCatalog = () => {
    if (!daemon) return;
    if (catalogInFlight) return;
    catalogInFlight = true;
    void fetchCatalog(daemon.url)
      .then((catalog) => {
        catalogInFlight = false;
        if (!catalog) return;
        let added = false;
        for (const s of catalog.sensors) {
          const k = kindKey(s.sensor_id, s.sensor_hash);
          const prev = sensorCatalog.get(k);
          if (
            sensorKinds.get(k) === s.kind &&
            prev?.sensor_entry_json === s.sensor_entry_json &&
            prev?.frame_entry_json === s.frame_entry_json
          ) {
            continue;
          }
          sensorKinds.set(k, s.kind);
          sensorCatalog.set(k, s);
          added = true;
        }
        if (!added) return;
        attachCachedKinds(realSensors);
        liveSensors = withVirtualWorldSensor(realSensors);
        strip.setSensors(liveSensors);
        strip.setToggled(toggled);
        pushTilesToStage();
        // Persisted-on audio toggles can only fire once we learn
        // which sensor is audio kind. Recheck whenever the catalog
        // brings new kinds in.
        reconcileAudioListenTarget();
      })
      .catch(() => {
        catalogInFlight = false;
      });
  };

  // Reconcile Park's audio listen target with whatever audio-kind
  // sensors are currently toggled on for this daemon. The audio
  // consumer subscribes to one peer at a time, so a "true" audio
  // toggle pins listen target = this daemon's peer_id; clearing
  // every audio toggle clears the target. Called on every toggle
  // change and on robot-view dispose.
  const reconcileAudioListenTarget = () => {
    if (!daemon) {
      setAudioListenTarget(null);
      return;
    }
    const hasAudioOn = liveSensors.some(
      (r) =>
        toggled.has(r.sensor_id) &&
        (sensorCatalog.get(kindKey(r.sensor_id, r.sensor_hash ?? ""))?.kind ??
          sensorKinds.get(kindKey(r.sensor_id, r.sensor_hash ?? ""))) === "audio",
    );
    setAudioListenTarget(hasAudioOn ? daemon.url : null);
  };

  const toggleSensor = (sensor_id: string) => {
    if (toggled.has(sensor_id)) toggled.delete(sensor_id);
    else toggled.add(sensor_id);
    strip.setToggled(toggled);
    pushTilesToStage();
    persistToggled();
    reconcileAudioListenTarget();
  };

  strip.onToggle(toggleSensor);

  // Prefer a camera sensor as the auto-toggle seed. Uses the
  // SDK-declared `entry.type` rather than guessing from the sensor_id —
  // substring matching mis-classifies `*_camera` pointcloud sensors
  // (e.g. bracketbot's depth_camera). On the first call after a live
  // session opens, registry entries may not have resolved yet; fall
  // through to the first row so the strip isn't empty.
  const pickDefaultSensor = (rows: SensorRow[]): SensorRow | undefined =>
    rows.find((r) => r.entry?.type === "camera") ?? rows[0];

  const maybeApplyDefaultToggle = (sensors: SensorRow[]) => {
    if (defaultToggleApplied || sensors.length === 0) return;
    const seed = pickDefaultSensor(sensors);
    if (!seed) return;
    toggled.add(seed.sensor_id);
    defaultToggleApplied = true;
    persistToggled();
  };

  // ─── sensor_logs + info polling ───────────────────────────────────
  let disposeState: () => void = () => {};
  let disposeInfo: () => void = () => {};
  if (daemon) {
    disposeState = subscribeSensorLogs(daemon.url, (state) => {
      lastState = state;
      sidebar.setSensorLogs(state);
      realSensors = sensorsFromState(state);
      attachCachedKinds(realSensors);
      liveSensors = withVirtualWorldSensor(realSensors);
      strip.setSensors(liveSensors);
      maybeApplyDefaultToggle(liveSensors);
      strip.setToggled(toggled);
      pushTilesToStage();
      refreshCatalog();
    });
    // /api/info gives us the daemon's libp2p peer_id, which the live
    // video tiles need to subscribe to grimsby streams. Until this
    // resolves, video tiles stay off the stage (per pushTilesToStage's
    // peerId gate).
    disposeInfo = subscribeInfo(daemon.url, (snap) => {
      const next = snap?.info.peer_id ?? null;
      if (next === peerId) return;
      peerId = next;
      pushTilesToStage();
    });
  }

  // ─── keyboard shortcuts ───────────────────────────────────────────
  const onKey = (e: KeyboardEvent) => {
    if (
      e.target instanceof HTMLInputElement ||
      e.target instanceof HTMLTextAreaElement ||
      (e.target instanceof HTMLElement && e.target.isContentEditable)
    ) {
      return;
    }
    if (e.metaKey || e.ctrlKey || e.altKey) return;

    if (e.key === "?" || (e.key === "/" && e.shiftKey)) {
      e.preventDefault();
      showCheatsheet(SHORTCUTS);
      return;
    }
    if (e.key >= "1" && e.key <= "9") {
      const idx = Number(e.key) - 1;
      const s = liveSensors[idx];
      if (s) {
        e.preventDefault();
        toggleSensor(s.sensor_id);
      }
      return;
    }
    const focused = stage.getFocusedTile();
    switch (e.key.toLowerCase()) {
      case "f":
        if (focused) {
          e.preventDefault();
          focused.toggleFreeze();
        }
        break;
      case "s":
        if (focused) {
          e.preventDefault();
          focused.snapshot();
        }
        break;
      case "c":
        if (focused) {
          e.preventDefault();
          focused.close();
        }
        break;
    }
  };
  window.addEventListener("keydown", onKey);

  return {
    el,
    dispose: () => {
      window.removeEventListener("keydown", onKey);
      disposeState();
      disposeInfo();
      stage.dispose();
      sidebar.dispose();
      // Stop the audio consumer's subscription when the operator
      // leaves the robot view. Without this Park would keep playing
      // K1 audio after navigating to the dashboard.
      setAudioListenTarget(null);
    },
  };
}

// Derive a sensor list from a daemon's /api/sensor_logs response.
// Sensors are inferred from `sensor_logs[].sensor_id` since the
// daemon doesn't ship a dedicated "sensors on this daemon" endpoint;
// every bound sensor in the live session has at least an auto-buffer
// running, so the union of `sensor_id` values is the sensor list.
function sensorsFromState(state: DaemonSensorLogs | null): SensorRow[] {
  if (!state) return [];
  const map = new Map<string, SensorRow>();
  for (const l of state.sensor_logs) {
    let row = map.get(l.sensor_id);
    if (!row) {
      row = {
        sensor_id: l.sensor_id,
        sensor_hash: l.sensor_hash,
        sensor_logs: [],
      };
      map.set(l.sensor_id, row);
    }
    row.sensor_logs.push(l);
  }
  return Array.from(map.values()).sort((a, b) =>
    a.sensor_id.localeCompare(b.sensor_id),
  );
}

type WorldSensorPair = {
  pointCloud: SensorRow;
  joint: SensorRow;
};

function worldSensorPair(rows: SensorRow[]): WorldSensorPair | null {
  const pointCloud = rows.find((r) => r.entry?.type === "point_cloud");
  const joint = rows.find((r) => isJointEncodersRow(r));
  if (!pointCloud || !joint) return null;
  return { pointCloud, joint };
}

function isJointEncodersRow(row: SensorRow): boolean {
  return (
    row.entry?.type === "joint_encoders" || row.sensor_id.endsWith("/joint_encoders")
  );
}

function withVirtualWorldSensor(rows: SensorRow[]): SensorRow[] {
  const pair = worldSensorPair(rows);
  if (!pair) return rows;
  return [
    {
      sensor_id: WORLD_SENSOR_ID,
      sensor_logs: [...pair.pointCloud.sensor_logs, ...pair.joint.sensor_logs],
    },
    ...rows,
  ];
}
