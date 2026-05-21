# auki-domain-browser

Browser Domain peer adapter for the Auki SDK.

This package is the browser sibling of `auki-domain-py`: a consumer-facing Domain peer handle for web apps. Park is the first consumer. The package owns browser peer identity, Discovery HTTP calls, participant snapshots, and the SDK boundary for future browser transport and sensor streams.

The package auto-picks a runtime peer from `window.aukiBrowserPeer.createPeer()` when available. With the current wasm adapter, browser peers can join a native WebRTC Direct Manager, observe pushed browser roster snapshots, and publish media presence intent. Without a healthy installed adapter, it exposes the Park-compatible contract with `joinDomain`, `createDomain`, and stream methods returning structured `transport_unavailable` errors.

The public contract follows the current SDK stream vocabulary: sensor kinds are `camera`, `point_cloud`, `joint_encoders`, `audio`, `detection`, and `unknown`; stream states are `off`, `idle`, `connecting`, `connected`, `reconnecting`, `declined`, and `error`.
