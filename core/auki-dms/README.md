# auki-dms

DMS task types and HTTP helpers for claiming tasks, sending heartbeats, and
reporting results. Creating a client does not start polling or execute tasks.
DMS owns task state and leases.

Use [`auki-tasks`](../auki-tasks/README.md) to run compute or robot handlers
with managed heartbeats and cleanup. For existing hosts, see the
[native runner adapters](../../docs/reference/tasks.md#native-runner-adapters),
including capability selection when updating an older SDK dependency.

For relay bookings, use [`auki-relay-booking`](../auki-relay-booking/README.md).
