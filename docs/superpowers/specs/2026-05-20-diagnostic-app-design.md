# Auki Diagnostic App Design

Status: draft design for user review.

Date: May 20, 2026.

## Goal

Build a smallest-useful standalone native diagnostic app for macOS and Linux that helps test Auki networking, clustering, and heartbeat-based time sync.

The primary physical test is two laptops standing next to each other, clustered, each flashing a large visual indicator once every three seconds. If time sync is working, the flashes should line up by eye.

The opening hypothesis is that Linux UTC and macOS UTC may differ enough to be visible. UTC mode answers: how aligned are the machines before Auki sync? Domain mode answers: did Auki `session -> domain` correction reduce the visible offset?

## Placement

The app lives under:

```text
examples/diagnostic-app/
```

It is an example application, not an SDK library component. It should not live under `crates/`.

The package may still be included as a Cargo workspace member so it can run with:

```bash
cargo run -p auki-diagnostic-app
```

## Platform And UI

Use Rust `eframe/egui` for the native UI. The first target platforms are macOS and Linux.

The UI is one native window:

- left sidebar: local peer identity, current cluster, join/create and leave controls;
- top status strip: networking, heartbeat freshness, and `session -> domain` sync status;
- central flash panel: large visual flash driven by the selected time basis;
- bottom diagnostics: peer rows and recent events.

Peer IDs must be displayed as suffixes, not prefixes. The UI should show `...<last 6 characters>` because the leading characters are often identical. Full peer IDs can be available later through copy/detail affordances, but the compact operational display uses the suffix.

## Time Sync Semantics

The app must avoid ambiguous "clock offset" wording.

The displayed sync estimate is:

```text
local session clock -> cluster domain clock
```

The domain clock follows the heartbeat time-sync design in `docs/superpowers/plans/2026-05-19-domain-clock-heartbeat-time-sync.md`: the target clock id is `"<cluster-name>/domain-clock"`. In v1, the domain clock is backed by the current Manager's session-monotonic clock. The Manager is therefore expected to show an approximately zero `session -> domain` offset, while followers show their estimated offset into that same domain clock.

The flash test starts in UTC mode. Every peer computes the next three-second UTC boundary and flashes from local UTC/wall-clock time. This lets two laptops be placed side by side to eyeball the raw offset before Auki domain-clock sync is trusted.

The flash panel has a timing-mode control:

- `UTC`: default mode; flashes every three seconds on UTC wall-clock boundaries, with no Auki correction.
- `Domain`: flashes every three seconds on cluster domain-clock boundaries, with Auki `session -> domain` correction applied.

In Domain mode, every peer computes the next three-second boundary in domain-clock time, converts that target to its own session/local time using the current estimate, and flashes when local time reaches the converted instant.

The flash runs automatically. There is no separate "Start Flash Test" button in v1. UTC mode can run before clustering. Domain mode requires the peer to be clustered and have a fresh sync estimate.

## Architecture

The app should be thin over SDK surfaces and should not introduce new protocol semantics.

Use three small layers:

1. `app_state` owns user config, current status snapshots, recent events, and display helpers.
2. `sdk_runtime` owns identity, swarm construction, `ClusterManager`, background polling, and conversion from SDK state into UI snapshots.
3. `ui` renders pure `egui` views and sends user actions to the runtime.

If the `session -> domain clock` snapshot API from heartbeat sync is not implemented yet, this diagnostic app depends on that work. The app should show sync as unavailable rather than faking synchronized flashes from local wall-clock time.

## First Version Scope

In scope:

- configure a Discovery URL and cluster name;
- join or create the named cluster;
- leave/shutdown the current cluster;
- show local peer diagnostics: display name, peer suffix, app id, role, and session clock id;
- show cluster diagnostics: cluster name, peer count, Manager suffix, peer rows, and recent events;
- show heartbeat and sync diagnostics: last heartbeat age, sync freshness, `session -> domain` offset, and unavailable/stale states;
- run the automatic three-second flash test in either UTC mode or Domain mode;
- default to UTC mode and provide a button/control to switch the indicator to Domain mode.

Out of scope:

- browser peers;
- polished installer packaging;
- multi-cluster browsing;
- stream, sensor, registry, pose, or map inspection;
- recording TimeTransform Logs from the diagnostic app;
- compensating for display hardware latency beyond showing local render timing diagnostics.

## Error Handling

The app should make failure states visible without hiding them behind retries:

- Discovery unavailable: show a clear disconnected state and keep join/create enabled.
- Cluster join/create failure: log the error in recent events and leave the app unclustered.
- No peers: show a valid one-peer cluster state, but sync/flash should indicate that cross-peer validation is not available.
- Sync unavailable or stale: UTC flashing remains available; Domain mode either becomes unavailable or renders in a clearly disabled/stale state.
- Manager handoff: update role, manager suffix, domain-clock status, and recent events.

## Testing

Unit tests should cover deterministic logic:

- peer-id suffix formatting;
- status derivation from runtime snapshots;
- sync freshness/staleness mapping;
- next three-second UTC boundary calculation;
- next three-second domain-clock boundary calculation;
- conversion from domain target time to local/session flash time.

Live two-laptop behavior is an integration/manual test:

1. Start Discovery.
2. Run the diagnostic app on laptop A and create/join the same cluster.
3. Run the diagnostic app on laptop B and join that cluster.
4. Stand the laptops next to each other in UTC mode and eyeball the raw flash offset.
5. Confirm both apps show two peers and a fresh `session -> domain` estimate.
6. Switch both apps to Domain mode and verify the large flash indicators fire together every three seconds.

## Open Dependency

The clean version of this app needs the heartbeat time-sync surface described in `docs/superpowers/plans/2026-05-19-domain-clock-heartbeat-time-sync.md`. If implementation begins before that surface lands, the first implementation plan should either:

- implement the needed heartbeat sync API first; or
- build the app shell and diagnostics with sync/flash marked unavailable until the API exists.
