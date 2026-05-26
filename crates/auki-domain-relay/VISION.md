# auki-domain-relay Vision

`auki-domain-relay` is the Domain Relay capability: a native- and browser-compatible reachability service for Auki Domains.

The Relay is separate from the Domain Manager. The Manager remains authoritative for membership and `/auki/join`; the Relay provides browser/native reachability by letting Managers reserve through a native relay address while browsers discover the Relay's WebSocket address. Domain-scoped reservation grants and dialing policy through manager/discovery-issued grants are pending.
