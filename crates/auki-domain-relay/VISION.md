# auki-domain-relay Vision

`auki-domain-relay` is a deployable reachability primitive: a native- and
browser-compatible libp2p Circuit Relay v2 server.

It remains deliberately separate from Domain authority. DDS credentials and
`auki-p2p` admission determine who may participate in a Domain; host-provided
routes determine how peers find the relay. The relay does not introduce a
Manager, cluster roster, leader election, or topology-synchronization model.
