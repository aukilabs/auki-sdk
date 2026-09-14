# Auki networking for Python

The `auki_sdk` module exposes `AukiSession` and `AukiPeer`. It requires
Python 3.8 or newer and Rust 1.89 or newer.

Build the binding from the SDK repository root:

~~~sh
cd core/bindings/python/auki-sdk-py
python3 -m venv .venv
. .venv/bin/activate
python -m pip install 'maturin>=1.5,<2.0'
maturin develop --locked --no-default-features
~~~

`AukiSession.login_dev` signs in to the development services. Call `start_peer`
with a Domain ID and identity file path; await `peer.shutdown()` when finished.

To run an app that sends and receives messages, use
[Python Echo](../../../examples/portable-echo/python/README.md).
Compile custom Rust protocols into the same Python extension as the SDK. See
[custom protocols](../../../../docs/how-to/protocols.md).

## Domain data without a peer

The same session exposes `domains()` for ordinary Domain discovery and portal
metadata, and `data(domain_id)` for data operations and pose reads. Close the data
client before closing the shared session. See [Domain data](../../../../docs/how-to/domain-data.md)
for permission boundaries, streaming limits and migration from Posemesh clients.

`login_dev` and `login_app_dev` accept optional `client_id=`. Data metadata is
returned as dictionaries and file data as `bytes`. `DomainDataError` retains the
HTTP `status` and error `kind`. Streaming callbacks are async; cancellation of
an asyncio operation also cancels the native transfer and pending callback.
Await `data.close()` to drain multipart cleanup before releasing its session.

After building the extension, run offline integration tests:

```sh
python -m pytest core/bindings/python/auki-sdk-py/python_tests/test_domain_data.py -q
```

With an approved dev account/Domain and `AUKI_EMAIL`, `AUKI_PASSWORD`,
`AUKI_DOMAIN_ID`, `AUKI_CLIENT_ID` set, the [file example](examples/domain_data.py)
generates a 17 MiB file, streams it up and back, verifies SHA-256 and deletes its
unique record. It requires Python 3.9+; optionally pass an input file path.

```sh
python core/bindings/python/auki-sdk-py/examples/domain_data.py
```
