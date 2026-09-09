# Apps, services, and workers

Use `auki-sdk` to connect your app to peers. Use a
[Posemesh runner](https://github.com/aukilabs/posemesh/tree/main/core/compute-node)
to execute DMS tasks. These are different responsibilities: a service can
exchange data with peers without becoming a task worker.

## User apps and backend services

User apps sign in with email/password or import a ZITADEL login. Backend
services can use an App access key and secret. These are common usage patterns;
a native app can also sign in as a user.

Each peer selects a Domain that its User or App can access. Neither login
requires a node wallet or stake. Keep App secrets on trusted backends.

For ZITADEL, your app completes login and supplies the session to the SDK.
Supply a known Domain ID; imported sessions cannot currently list Domains.
See [authentication](../how-to/authenticate.md).

## Compute workers

A compute worker uses DDS node registration and wallet signatures (SIWE).
DDS checks the node's staking status; deployments can explicitly allow
unstaked registration. Check the requirements of your DDS environment.

DMS selects tasks matching the node's registered capabilities and mode:

- **Public:** public tasks across organizations and Domains, preferring the
  node's organization.
- **Dedicated:** tasks within the node's organization, across its Domains.
  Dedicated tasks take priority over that organization's public tasks.

DMS supplies access to the task's Domain through the lease. A node registration
does not grant unrestricted access to Domains.

See [Posemesh compute setup](https://github.com/aukilabs/posemesh/tree/main/core/compute-node#configuration-surface).

## Robot workers

A robot uses the same Posemesh task engine through its robot entrypoint.
It authenticates with a robot registration credential provisioned in DDS,
without a wallet or staking. If P2P is enabled, it also needs a separate
network identity key.

DDS assigns the robot to one Domain in its organization. DMS only gives it
matching **dedicated** tasks in that Domain. An unassigned robot can report
presence but cannot claim tasks. Reassignment requires the robot to be offline
and its active leases and tokens to expire.

See [Posemesh robot setup](https://github.com/aukilabs/posemesh/tree/main/core/compute-node#hello-runner-entrypoints).

## Task execution

Implement a Posemesh `Runner` for the task capability you support. The shared
engine handles polling, heartbeats, and reporting results to DMS.

DMS currently allows **one active task lease per node**, for both compute and
robot workers. A robot runner must keep physical task execution exclusive and
reject new tasks while busy.

P2P carries data; tasks go through DMS. Registering a P2P message handler does
not register a DMS task capability or change task scheduling.
