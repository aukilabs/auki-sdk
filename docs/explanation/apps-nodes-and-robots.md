# Apps, services, compute nodes, and robots

Apps, backend services, compute nodes, and robots use different credentials
and receive different access to Domains. A P2P connection lets them exchange
application data; DMS decides which compute node or robot may execute a task.

## User apps and backend services

User apps sign in with email/password or import a ZITADEL login. Backend
services can use an App access key and secret. These are common usage patterns;
a native app can also sign in as a user.

Each peer selects a Domain that its User or App can access. Neither login
requires a node wallet or stake. Keep App secrets on trusted backends.

For ZITADEL, your app completes login and supplies the session to the SDK.
Owner and scoped User grants support the Domain picker through the existing
User listing contract. Viewer listing needs the human Domain-allowlist exchange;
a known Domain can use its separate data authorization.
See [authentication](../how-to/authenticate.md).

## Compute nodes

A compute node uses DDS node registration and wallet signatures (SIWE).
DDS checks the node's staking status; deployments can explicitly allow
unstaked registration. Check the requirements of your DDS environment.

DMS selects tasks matching the node's registered capabilities and mode:

- **Public:** public tasks across organizations and Domains, preferring the
  node's organization.
- **Dedicated:** tasks within the node's organization, across its Domains.
  Dedicated tasks take priority over that organization's public tasks.

DMS supplies access to the task's Domain through the lease. A node registration
does not grant unrestricted access to Domains.

Use [SDK task handlers](../how-to/run-compute-tasks.md#start-a-compute-node)
or a [Posemesh compute runner](https://github.com/aukilabs/posemesh/blob/main/docs/how-to/configure-workers.md#compute-node)
to execute work.

## Robots

A robot authenticates with a registration credential provisioned in DDS,
without a wallet or staking. If P2P is enabled, it also needs a separate
network identity key.

DDS assigns the robot to one Domain in its organization. DMS only gives it
matching **dedicated** tasks in that Domain. An unassigned robot can report
presence but cannot claim tasks. Reassignment requires the robot to be offline
and its active leases and tokens to expire.

Use [SDK task handlers](../how-to/run-compute-tasks.md#use-a-robot)
or a [Posemesh robot runner](https://github.com/aukilabs/posemesh/blob/main/docs/how-to/configure-workers.md#robot)
to execute work.

## Task execution

An SDK handler implements a capability in Rust or Python. The managed runtime
handles registration, authentication, polling, heartbeats, and results.
Posemesh runners provide application-specific execution and storage interfaces.
See [Run compute and robot tasks](../how-to/run-compute-tasks.md).

A compute node receives Domain access for its active lease. Its optional peer
closes when that task ends. A robot can read its assigned Domain and stay
connected while idle, but writes require task authority. Ending a robot task
revokes its task data access while leaving its peer connected. Runtime shutdown
releases the peer, discovery registration, and relay bookings.

DMS currently allows **one active task lease per node**, for both compute nodes
and robots. A robot runner must keep physical task execution exclusive and
reject new tasks while busy.

P2P carries data; tasks go through DMS. Registering a P2P message handler does
not register a DMS task capability or change task scheduling.
