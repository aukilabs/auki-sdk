# auki-dms

DMS task types and HTTP helpers for polling tasks, sending heartbeats, and
reporting results. Creating a client does not start polling or execute tasks.

DMS is the source of truth for robot and compute task state.
[Posemesh runners](https://github.com/aukilabs/posemesh/tree/main/core/compute-node)
handle task execution; see the [task rules](../../docs/explanation/networking.md#robot-and-compute-tasks).

For relay bookings, use [`auki-relay-booking`](../auki-relay-booking/README.md).
