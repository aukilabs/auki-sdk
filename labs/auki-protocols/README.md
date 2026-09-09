# auki-protocols

Optional, **experimental** application protocols. The SDK does not require
them. This crate will also hold the protocols we select and freeze as stable.

The Rust crate has no default features. Some language bindings include these
protocols, including builds with `standard-protocols`. Your app must still
register the handlers it wants to serve.

To define your own messages, see
[Use a custom application protocol](../../docs/how-to/protocols.md).
