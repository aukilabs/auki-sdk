# auki-domain-browser

Browser Domain peer adapter for the Auki SDK.

This package is the browser sibling of `auki-domain-py`: a consumer-facing Domain peer handle for web apps. Park is the first consumer. The package owns browser peer identity, Discovery HTTP calls, participant snapshots, and the SDK boundary for future browser transport and sensor streams.

The first tranche intentionally fails closed for real peer transport. It can install `window.aukiBrowserPeer.createPeer()` and expose the Park-compatible contract, but `joinDomain`, `createDomain`, and stream methods return structured `transport_unavailable` errors until an SDK-owned browser transport is implemented.

The public contract follows the current SDK stream vocabulary: sensor kinds are `camera`, `point_cloud`, `joint_encoders`, `audio`, `detection`, and `unknown`; stream states are `off`, `idle`, `connecting`, `connected`, `reconnecting`, `declined`, and `error`.
