# auki-relay-booking

Strict, bounded client and validated wire types for the DMS relay-booking API.

This crate owns the requester-side HTTP contract. It does not select a relay,
run a libp2p node, recover a reservation, or supervise application authority.
Those lifecycle decisions belong to the native or browser SDK facade.
