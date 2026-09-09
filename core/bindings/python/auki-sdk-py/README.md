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
