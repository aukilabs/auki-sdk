# auki-dms

DMS task types and HTTP helpers for polling tasks, sending heartbeats, and
reporting results. Creating a client does not start polling or execute tasks.

DMS is the source of truth for robot and compute task state.
[`auki-tasks`](../auki-tasks/README.md) adds native compute handler execution.
[Posemesh runners](https://github.com/aukilabs/posemesh/tree/main/core/compute-node)
handle task execution; see the [task rules](../../docs/explanation/networking.md#robot-and-compute-tasks).

For relay bookings, use [`auki-relay-booking`](../auki-relay-booking/README.md).

`claim(capability)` preserves the filter and distinguishes leased/no-work/busy.
`lease_by_capability` also now filters; `lease_any()` explicitly lets DMS select
across authorized capabilities. See the [Posemesh upgrade requirement](../../docs/how-to/run-compute-tasks.md#rust-and-existing-posemesh-hosts).
