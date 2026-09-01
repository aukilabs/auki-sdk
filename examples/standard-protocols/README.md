# Standard protocol playground

This playground starts the same SDK-owned protocols on native Rust, Python,
Web, and Swift/iOS. Use it when building an application on Auki's standard
protocols or when checking cross-platform compatibility.

For a small example of **authoring a custom portable protocol**, use
[`portable-echo`](../portable-echo/README.md) instead.

## What it runs

Every peer opts in to both the client and serving roles for all six protocol
families:

| Family | Wire version | Fixture exercised |
| --- | --- | --- |
| Info | v1 | participant metadata |
| Catalog | v3 resources + v4 maps | one Message channel and an empty map list |
| Registry | v3 | one Frame registry entry |
| Blob | v1 | one content-addressed byte string |
| Message | v1 | one typed, timestamped message |
| Stream | v2 | one scalar sample |

Catalog owns two wire protocols, so each peer card advertises seven exact
protocol IDs while the test reports six protocol-family results.

The hosts are deliberately thin. Protocol contracts, bounds, authenticated
streams, and relay-backed peer lifecycle stay in Rust; TypeScript, Python, and
Swift only provide fixture data and application flow.

## Protected four-peer matrix

The matrix starts four distinct relay-backed peers and probes these directed
edges:

| Source → Target | Native | Python | Browser A | Browser B |
| --- | --- | --- | --- | --- |
| Native | — | ✓ | ✓ | — |
| Python | ✓ | — | — | ✓ |
| Browser A | ✓ | — | — | ✓ |
| Browser B | — | ✓ | ✓ | — |

That is eight directed edges × six protocol families = **48 checks**. Browser
A → Browser B and Browser B → Browser A are explicit cases; they are not
inferred from browser/native coverage.

Prerequisites:

- Rust and the `wasm32-unknown-unknown` target
- `wasm-pack`
- Node.js and npm
- Python 3.9 or newer
- `maturin`, or `uv` so the runner can invoke `uvx maturin`
- Playwright Chromium (`npx playwright install chromium`)
- one Auki User with access to the selected dev Domain

Run from the SDK checkout:

```sh
cd examples/standard-protocols/web
npm ci
npx playwright install chromium

export AUKI_EMAIL='developer@example.com'
export AUKI_PASSWORD='...'
export AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000'
npm run smoke:dev
```

Use `npm run smoke:dev -- --list` to print the 48 cases without credentials,
or `npm run smoke:dev -- --headed` to watch the browser peers. Success ends
with:

```text
AUKI_PROTOCOL_MATRIX_OK peers=4 edges=8 protocols=6 cases=48
```

The success marker is emitted only after all four peers, their relay bookings,
Chromium, and temporary state have shut down cleanly.

## Swift/iOS interoperability gate

The [Swift playground](swift/README.md) uses the same seven wire IDs and six
family checks. Its simulator gate runs all six protocols in both Swift/native
directions and both Swift/browser directions. This remains separate from the
four-peer protected matrix so the ordinary browser job does not require Xcode.

## Try the browser node manually

```sh
cd examples/standard-protocols/web
npm ci
npm run dev
```

Open the printed loopback URL in two tabs. In each tab:

1. log in with the same User;
2. select the same Domain;
3. start the peer;
4. copy the other tab's peer card; and
5. select **Probe all six**.

Each tab gets a distinct ephemeral Peer ID. Repeat in the opposite direction
to exercise the unique browser-to-browser case both ways. A native or Python
peer card can be pasted into the same target field.

## Native and Python control contract

The native and Python nodes are long-running matrix agents. Their stdout is
JSON Lines so the runner can exchange peer cards and commands; diagnostics go
to stderr.

Ready event:

```json
{"event":"ready","runtime":"native","card":{"version":1,"domainId":"...","peerId":"...","protocols":[],"routes":{"tcp":"...","wss":"..."}}}
```

Probe command and result:

```json
{"id":"native-to-python","command":"probe_all","target":{"version":1,"domainId":"...","peerId":"...","protocols":[],"routes":{"tcp":"...","wss":"..."}}}
{"event":"probe_result","id":"native-to-python","ok":true,"checks":{"info":true,"catalog":true,"registry":true,"blob":true,"message":true,"stream":true},"errors":{}}
```

The playground exchanges explicit peer cards because relay allocation is not
peer discovery. Native and Python dial the card's exact TCP circuit route; Web
dials its exact WSS circuit route. Every connection still authenticates the
expected Peer ID and selected Domain before protocol data is exposed.
