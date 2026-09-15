# auki-domain-client

Find Domains and read or write Domain data using the SDK's shared credentials.
User and App sessions work without starting a peer. Native task leases and
robot credentials use the same data client through `auki-tasks`.

The `auki-sdk` facade reexports these APIs for Rust, with bindings for Web and
Python. Start with [Work with Domain data](../../docs/how-to/domain-data.md).
See the [reference](../../docs/reference/domain-data.md) for credential support,
portal and pose reads, transfer limits, and multipart upload behavior.

Run local fixture tests and check WASM compilation from the repository root:

~~~sh
cargo test --locked -p auki-auth -p auki-domain-client -p auki-sdk
cargo check --locked --target wasm32-unknown-unknown -p auki-auth -p auki-domain-client -p auki-sdk
~~~
