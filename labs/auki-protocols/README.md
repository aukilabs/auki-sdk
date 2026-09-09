# auki-protocols

Optional, **experimental** application protocols, kept in `labs/` for now.
The networking engine does not require these implementations or mount them
automatically.

This crate is the intended home for the protocols selected and frozen as
stable. Its current versioned implementations do not carry that commitment.

The Rust crate has no default features. Some language bindings bundle protocol
features; an application still chooses which endpoints to mount.

To build your own protocol, see
[Use a custom application protocol](../../docs/how-to/protocols.md).
