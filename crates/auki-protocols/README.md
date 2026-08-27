# auki-protocols

Transport-neutral wire contracts for the authenticated Auki application
protocols. The crate owns exact protocol identifiers, request/response types,
bounded framing, validation, and locked wire vectors. It does not own
transport, authentication, protocol registration, providers, or task
lifecycle.

No protocol family is compiled by default. Enable only the families a binary
needs: `info`, `catalog`, `registry`, `blob`, `message`, and `stream`. Features
make wire contracts available; they do not install handlers. A Domain also
opts in to each exact inbound version through `ServedProtocols`, whose default
serves none.
