# Portable Echo in Python

Requires Python 3.8+, Rust 1.89+, and the development User/Domain from the
[networking tutorial](../../../../docs/tutorials/first-peer.md).

From the SDK repository root:

~~~sh
cd core/examples/portable-echo/python
python3 -m venv .venv
. .venv/bin/activate
python -m pip install 'maturin>=1.5,<2.0'
maturin develop --locked
~~~

Set `AUKI_EMAIL`, `AUKI_PASSWORD`, and `AUKI_DOMAIN_ID` as in the tutorial.
Start a serving peer:

~~~sh
export AUKI_IDENTITY_FILE='/tmp/auki-python-a/peer.identity'
python main.py
~~~

In another terminal, activate the same virtual environment and set the same
credentials and Domain. Use a different identity:

~~~sh
export AUKI_IDENTITY_FILE='/tmp/auki-python-b/peer.identity'
python main.py --discover '<Peer ID from the serving terminal>'
~~~

Expect `echo: hello from Auki`. Stop the server with Ctrl-C.

The [Python app](main.py) uses the shared Rust endpoint through one extension;
it does not implement a second wire codec.
