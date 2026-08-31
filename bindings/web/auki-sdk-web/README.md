# auki-sdk-web

Temporary compatibility bridge for the current Rust/Wasm example.

The canonical `auki-sdk` crate owns browser authority renewal, mandatory WSS
relay booking, the authenticated peer runtime, managed application protocols,
exact-route streams, and ordered shutdown. New Rust/Wasm code should use its
`AukiPeer`, `AukiPeerConfig`, `AukiPeerProtocols`, and `AukiProtocolSpec`
surface directly. This bridge only keeps the existing portable-echo app
building until its application adapter moves to that surface.

App access keys and secrets are deliberately unsupported in the browser. Use
User authentication or short-lived authority issued by a trusted backend.
