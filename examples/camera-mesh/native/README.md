# Native Rust Camera Mesh

This headless peer can publish the checked-in deterministic 480×270 JPEG or
consume another Camera Mesh publisher. It composes the application in Rust and
uses one JSON object per line on stdin and stdout.

From the SDK root, configure either User credentials:

```sh
export AUKI_EMAIL='developer@example.com'
export AUKI_PASSWORD='...'
```

or trusted App credentials:

```sh
export AUKI_APP_ACCESS_KEY='...'
export AUKI_APP_SECRET='...'
```

Then select a Domain and give each process a different persistent identity
file. Start a publisher:

```sh
AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000' \
AUKI_IDENTITY_FILE='.state/camera-publisher.identity' \
AUKI_CAMERA_ROLE='publisher' \
cargo run --locked -p auki-camera-mesh-native
```

Start a viewer in another terminal:

```sh
AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000' \
AUKI_IDENTITY_FILE='.state/camera-viewer.identity' \
AUKI_CAMERA_ROLE='viewer' \
cargo run --locked -p auki-camera-mesh-native
```

The first stdout line is a `ready` event containing the peer card. Copy whole
JSON commands into stdin to discover publishers, try a view, approve the
resulting request on the publisher, retry, send controls, fetch a snapshot, or
shut down. See the
[shared Camera Mesh guide](../README.md#native-and-python-jsonl-contract) for
the command shapes.

For one bounded acceptance run against an approved live publisher, send:

```json
{"command":"exercise_live","id":"live","target":<PUBLISHER_CARD>,"requestId":"live-snapshot"}
```

The viewer keeps one Stream subscription open while it receives two distinct
camera captures, pauses, drains buffered frames until the feed becomes quiet,
resumes to a newly captured frame, and fetches a SHA-256-verified snapshot. The
result is one `exercise_live_result` object.

`AUKI_NODE_NAME` changes the participant name. Publishers default to
`AUKI_DISCOVERY_MODE=discover_and_advertise`; viewers default to
`discover_only`.
