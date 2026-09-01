# Python standard-protocol node

This long-running example mounts and probes all six standard protocol families
through the Rust-backed `auki_sdk` extension. It is primarily driven by the
parent playground's matrix smoke test; its stdout is a JSONL control surface.

Build the local extension into an active virtual environment, then run the
node with a unique identity file:

```bash
maturin develop --manifest-path ../../../bindings/python/auki-sdk-py/Cargo.toml

AUKI_EMAIL='developer@example.com' \
AUKI_PASSWORD='...' \
AUKI_DOMAIN_ID='...' \
AUKI_IDENTITY_FILE='./state/python.identity' \
python -u main.py
```

See the parent [playground guide](../README.md) for the JSONL contract and the
complete native/Python/browser matrix.
