# auki-network Binding Surface

This file is the binding coverage contract for `auki-network`. Every required
operation listed here must have a matching `// binding-surface: ...` marker in
the crate tests before the implementation phase that activates the test.

## Native UniFFI Required

- Peer identity and derivation.
- Runtime lifecycle.
- Runtime control.
- Event draining.
- Request/response protocols.
- Discovery client.
- App-instance derivation.
- Byte streams.
- Diagnostics.

## Browser JavaScript Required

- Peer identity and derivation.
- Protocol constants.
- Browser probe.
- Message protocol.
- Request/response DTO encoding helpers.
- JavaScript-owned libp2p peer facade.
