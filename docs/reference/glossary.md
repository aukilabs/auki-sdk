# Glossary

Terms used by the core SDK and its [networking guides](../explanation/networking.md).
For APIs and platform support, see the [networking reference](networking.md).

## Peers and access

### AukiPeer

The networking runtime inside your app. One `AukiPeer` owns a Peer ID and its
connections in one selected Domain. It coordinates authentication renewal,
relay bookings, optional discovery, and shutdown.

### Peer ID

The libp2p identifier derived from a peer's network keypair. Saving the key
keeps the ID across restarts. Each running peer needs its own key. A Peer ID
identifies a transport peer; it does not by itself grant backend access.

### Domain and Domain ID

A Domain represents a physical space and its access policy in DDS. Its Domain
ID is a UUID. Peers select a Domain and authenticate within it; applications
still decide which operations each peer may perform.

### User and App

A **User** is a person signing in with an Auki account or an imported ZITADEL
login. An **App** is an Auki application identity that a trusted backend uses
with an access key and secret. An App credential is separate from DDS
credentials for a compute node or robot. See [authentication](../how-to/authenticate.md).

### Authentication session

A login and its renewable credentials, reused when starting peers. An imported
ZITADEL session has one refresh owner and requires the app to persist replacement
credentials. On logout, stop its peers, close the session, then delete saved
credentials. See [session handling](../how-to/authenticate.md#reuse-a-zitadel-login).

### P2P credential

A signed credential issued by DDS that binds a backend identity to a Peer ID,
Domain, scopes, and expiry. Peers verify it when authenticating a connection.
It does not grant every application operation or Domain Server write.

### Authenticated peer

The verified remote identity available to a protocol handler through
`stream.remote_peer()`. Use it when checking your app's permissions. Native
`known_peers()` reports authenticated connections; it is not an authorization
list.

## Services

### API

The Auki backend for User and App authentication and service-token exchange.
It is configured separately from DDS and DMS. Here, “API” names that service;
“SDK API” refers to the library's public interfaces.

### DDS

Domain Discovery Service. It manages Domain access, compute node and robot
identity, P2P authorization, and optional peer discovery. It issues the
credentials used to authenticate peers in a Domain.

### DMS

Domain Manager Service. It owns task state, scheduling, and task leases. It
also provides relay booking through a separate contract. Booking a relay does
not register a task capability or start task execution.

### Domain Server

The service that stores and serves Domain data. Applications need authorization
for the data operations they perform; joining a Domain over P2P does not grant
arbitrary data access.

## Connections and messages

### P2P

Peer-to-peer communication between apps, directly or through a relay. It
carries application data; DMS handles task submission and state.

### Discovery

Finding peer IDs, supported protocols, and addresses. Apps can use their own
source or opt into DDS discovery, which is off by default. Discovery results
help locate a peer; the connection still needs authentication.

### Route and multiaddr

A route is a peer address encoded as a libp2p multiaddr. Native peers use TCP
addresses; browsers use WSS relay addresses. A route is a location hint, so
always connect with the expected Peer ID.

### Relay and relay booking

A relay forwards connections so a peer can accept them without a public port.
The SDK books, renews, and releases relay capacity through DMS. Relay use and
discovery are separate choices. This is the SDK's libp2p relay path; legacy
HDS/Hagall networking has a different contract.

### Application protocol and protocol ID

The message format and conversation agreed on by two apps. A protocol ID names
that contract, such as `/example/echo/1.0.0`. Incompatible changes require a new
ID. The SDK opens authenticated streams; the protocol's code validates and
handles the bytes.

### Client, handler, and endpoint

A **client** makes outgoing requests. A **handler** serves incoming requests
for a registered protocol. An **endpoint**, such as Echo's `EchoEndpoint`, owns
that registration and its cleanup. Keep it alive while serving and close it
before shutting down the peer. See [custom protocols](../how-to/protocols.md).

## Compute nodes and robots

### Compute node

A participant that uses DDS node registration and a signing wallet to process
eligible DMS tasks through a Posemesh runner. DDS applies staking requirements;
task access depends on the node's capabilities and public or dedicated mode.
See [compute nodes](../explanation/apps-nodes-and-robots.md#compute-nodes).

### Robot

A participant with DDS robot credentials that can be assigned to one Domain.
Its Posemesh runner executes eligible dedicated DMS tasks in that Domain.
Robot registration does not require a node wallet or stake. See
[robots](../explanation/apps-nodes-and-robots.md#robots).

### Runner

The software that executes a task capability on a compute node or robot.
Posemesh's shared task engine handles polling, heartbeats, and reporting results
to DMS. Registering a P2P handler does not register a runner's task capability.

### Task and task lease

A **task** is work submitted through DMS. A **task lease** gives a compute node
or robot temporary authority to execute it, including the task's Domain access.
DMS owns the lease and task state; the SDK's P2P transport carries application
data. See [task execution](../explanation/apps-nodes-and-robots.md#task-execution).
