# Auki networking for Python

The `auki_sdk` module exposes `AukiSession` and `AukiPeer`. It requires
Python 3.8 or newer and Rust 1.89 or newer.

Build the networking facade from the SDK repository root:

~~~sh
cd core/bindings/python/auki-sdk-py
python3 -m venv .venv
. .venv/bin/activate
python -m pip install 'maturin>=1.5,<2.0'
maturin develop --locked --no-default-features
~~~

`AukiSession.login_dev` creates a User session; `start_peer` accepts the selected
Domain and identity path. Await peer shutdown when finished.

For a runnable custom-protocol app, use
[Python Echo](../../../examples/portable-echo/python/README.md).
Custom Rust protocol adapters must share the Python extension that owns the
peer. See the [networking reference](../../../../docs/reference/networking.md).
