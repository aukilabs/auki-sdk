# Python Camera Mesh

This headless peer can publish a checked-in animated 480×270 test feed or
consume another Camera Mesh publisher. The application uses the Rust-backed
`auki_sdk` extension and needs no camera, Pillow, OpenCV, or platform image
codec.

## Install the local binding

From the SDK root:

```sh
python3 -m venv .venv-camera-mesh
source .venv-camera-mesh/bin/activate
maturin develop --locked --manifest-path bindings/python/auki-sdk-py/Cargo.toml
```

If `maturin` is not installed directly:

```sh
uvx maturin develop --locked --manifest-path bindings/python/auki-sdk-py/Cargo.toml
```

Configure either User credentials:

```sh
export AUKI_EMAIL='developer@example.com'
export AUKI_PASSWORD='...'
```

or trusted App credentials:

```sh
export AUKI_APP_ACCESS_KEY='...'
export AUKI_APP_SECRET='...'
```

## Start two peers

Start a publisher:

```sh
AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000' \
AUKI_IDENTITY_FILE='.state/python-camera-publisher.identity' \
AUKI_CAMERA_ROLE='publisher' \
python -u examples/camera-mesh/python/main.py
```

Start a viewer in another terminal with the same credentials and Domain but a
different identity file:

```sh
AUKI_DOMAIN_ID='00000000-0000-0000-0000-000000000000' \
AUKI_IDENTITY_FILE='.state/python-camera-viewer.identity' \
AUKI_CAMERA_ROLE='viewer' \
python -u examples/camera-mesh/python/main.py
```

Each process prints a `ready` event containing its peer card. Send `discover`
and `view` commands to the viewer. The first view causes an `approval_required`
event on the publisher; send `approve` there, then retry the view. Pause,
resume, and snapshot are also viewer commands.

See the
[shared Camera Mesh guide](../README.md#native-and-python-jsonl-contract) for
the exact JSONL commands. Discovery candidates can be passed as targets; the
peer selects their native TCP relay route before dialing.

`AUKI_NODE_NAME` changes the participant name. Publishers default to
`AUKI_DISCOVERY_MODE=discover_and_advertise`; viewers default to
`discover_only`. `AUKI_CAMERA_AUTO_APPROVE=1` makes an unattended demo
publisher admit any authenticated viewer in its Domain; it is disabled by
default. The [batch launcher](../README.md#try-it) configures this mode for you.
Set `AUKI_CAMERA_FRAME_MODE=still` only when a byte-stable JPEG fixture is
needed by an automated interoperability test.
