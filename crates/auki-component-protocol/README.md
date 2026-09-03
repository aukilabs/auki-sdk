# Auki Component Protocol

Standalone, portable application protocols that expose the network-independent
`auki-components` model through mutually authenticated `AukiPeer` streams.

The protocol family is intentionally separate from the manager-era
`auki-protocols` crate:

- `/aukilabs/components/catalog/1.0.0`
- `/aukilabs/components/observations/1.0.0`
- `/aukilabs/components/operations/1.0.0`

The authenticated stream supplies the caller peer identity. Wire messages may
identify a caller Component, but they cannot assert or override the caller peer.
