# Auki Effector Control

## Status

Idea-space design note. This document captures the intent and current direction of a design discussion. It is not yet a wire protocol, implementation plan, or compatibility commitment.

## Motivation

The Auki SDK lets peers advertise and stream reporting Resources, but it does not yet provide a general, permissioned way for one peer to advertise or use a control capability.

The Resource catalog should evolve from a catalog of requestable logs into one discovery surface for all typed network capabilities. Control should use the same log-native Resource model: a controller publishes intentions to a live Resource, and an authorized controlled peer follows that Resource.

The immediate use case is a Galbot G1 that exposes a small ambient-behavior control endpoint. A paired controller application should be able to publish an ambient movement target, consume the robot's response to that intention, and observe the robot's actual state through its normal SDK Resources.

The design should be general enough to cover other things that affect the world, including:

- Robot locomotion
- Arms and grippers
- Speakers
- Displays
- Light indicators
- Higher-level robot behaviors



## Working Model

A controller application exposes a control-producer Resource. The Resource is a live intent log containing the compatible intentions the controller originates. A controlled component exposes a control-consumer Resource that identifies which intent profiles it accepts and which outcome and reporting Resources describe its response.

```text
Controller peer
|-- control producer: ambient-controller
|   `-- live intent log: ambient_control.v1
`-- follows: behavior/ambient/outcomes

Controlled peer / device
`-- behavior/ambient
    |-- control consumer
    |   `-- follows paired live origin: ambient-controller
    |-- outcome Resource: behavior/ambient/outcomes
    `-- reporting Resources: behavior-status, joint, and pose
```

The same pattern applies independently to locomotion, manipulation, speakers, displays, lights, and higher-level behaviors. Each may expose a control consumer with its own accepted profiles, arbitration policy, outcome Resource, and reporting Resources.

The control loop is log-native and asynchronous:

```text
control-producer intent log
    -> followed by paired control consumer
    -> consumer outcome Resource
    -> consumed by control producer
    -> measured-state reporting Resources
```

All capabilities are discovered through the Resource catalog. There is no second direct command-delivery path.

> One catalog, typed Resources, and logs as the control transport.



## Terminology



### Resource

A typed, independently addressable capability that a peer exposes to the network. A Resource descriptor identifies the capability and the interface or schema it implements.

### Reporting Resource

An SDK data product that reports observed state. Reporting Resources remain independent from control sessions and may be observed, recorded, or materialized by peers that have no control authority.

### Control-producer Resource

A live intent log owned by a controller peer. Its descriptor identifies the control profiles and schema versions represented by its entries. A paired control consumer follows the producer's current live origin to receive intentions. Other authorized peers may consume the same log for diagnostics.

Advertising or reading a producer Resource does not prove authority to control anything.

### Control-consumer Resource

A Resource owned by a controlled peer that identifies an executor of authorized intentions. It describes the semantic profiles it accepts, its arbitration and disconnect policies, the outcome Resource it publishes, and the reporting Resources that reveal physical effects.

The consumer follows the current live origin of a paired producer Resource. It never actuates from a historical, relayed, or materialized copy. A control consumer is also referred to as a control endpoint in this note.

### Outcome Resource

A live log published by the controlled peer with events correlated to producer intent entries. It records whether the consumer received, authorized, accepted, rejected, is executing, completed, or failed an intention.

An outcome Resource reports the consumer's protocol and execution decisions, not measured physical state. A control producer should consume the outcome Resource referenced by the consumer it controls.

### Controller

An SDK peer that owns a control-producer Resource, obtains a pairing grant for a compatible control-consumer Resource, and consumes the consumer's outcome and measured-state Resources.

### Pairing grant

The authorization that binds a named control-producer Resource on an authenticated controller peer to a named control-consumer Resource and selected operations on an authenticated controlled peer.

### Control session

A live, paired following relationship between a control-producer Resource and a control-consumer Resource under a named session epoch. The consumer follows the producer's live intent-log head, while the producer follows the current authenticated live origin of the consumer's outcome Resource. The consumer's arbitration policy determines whether the relationship is exclusive, shared, queued, mixed, composited, or last-writer-wins.

A control session is an authorization, arbitration, epoch, and liveness boundary. It is not an alternative command transport outside the Resource streams.

The control-session epoch is distinct from the SDK recording Session and its Session ID.

### Control lease

Temporary authority, issued by the consumer's arbitration policy, that makes a paired producer epoch eligible for actuation. An exclusive consumer grants a lease to at most one producer epoch at a time. A lease is neither a pairing grant nor a command transport.

## Architectural Boundaries



### One Resource catalog, log-native control

The current Resource catalog describes requestable logs with log-specific concepts such as source and writer peers, live or sealed state, retained extent, available entries, and manifest references. Control-producer and outcome Resources are logs and naturally share many of those concepts. A control-consumer Resource is an independently addressable capability but not itself a log.

This is a limitation of the current `ResourceEntry` shape, not of the Resource concept. The catalog should become genuinely variant-oriented: common fields identify an independently addressable capability, while reporting and control variants carry only the fields appropriate to them.

Conceptually:

```text
Resource
|-- reporting
|   |-- sensor_log
|   |-- pose_log
|   |-- time_transform_log
|   `-- detection_log
`-- control
    |-- control_producer: live intent log
    |-- control_consumer: executor capability
    `-- outcome Resource: consumer decision log
```

The Resource catalog remains the single discovery surface, and Resource streams carry both intentions and outcomes. The consumer opens and follows the paired producer's intent log. The producer opens and should follow the outcome Resource referenced by the consumer from its current authenticated origin. Reporting Resources independently carry observations of physical state.

The intent and outcome logs are both transport and record. Diagnostics consume the same records used by the control loop; there is no optional mirror that can diverge from a separate direct command path.

### Live-origin actuation boundary

The existence of an intent entry in a log is not sufficient authority to actuate. A control consumer may act only on an entry that satisfies all of these conditions:

1) The producer peer and Resource are the exact source tuple named by an active pairing grant.

2) The entry arrives while following that producer Resource's current live origin directly from its authenticated owner.

3) The entry belongs to the session epoch currently accepted by the consumer's arbitration policy.

4) Its command ID has not been processed, its sequence is strictly greater than the last accepted sequence in that epoch, and its expiry has not passed.

5) The requested operation and interface version are within the pairing grant.

Historical reads and backfill are never eligible for actuation. Neither are entries read from sealed, relayed, cached, or materialized copies, even when those copies preserve the original `source_peer_id`. Such entries remain useful for diagnostics and audit.

The live-origin rule separates availability from authority. Authorized diagnostic peers may read and materialize intent and outcome logs, but only the paired consumer following the authenticated origin under its active epoch may interpret new entries as actionable.

The producer should likewise consume outcomes from the consumer's current authenticated live origin when making current control decisions. Historical, backfilled, sealed, relayed, cached, or materialized outcomes remain valid diagnostic records, but do not establish the consumer's current control state.

### Epochs, ordering, and replay protection

Every intent needs:

- A stable command ID
- A session epoch
- A sequence number monotonic within that epoch
- An issue time and explicit expiry
- An authenticated producer origin established by the live transport,
  per-entry authentication, or both

The consumer must retain enough command and sequence state across reconnects and restarts to reject duplicates. Reopening a stream does not make earlier entries actionable, and a new epoch does not revive commands from an old one. The exact epoch-establishment and crash-consistency mechanisms remain protocol design questions.

Loss of the live origin stream ends its liveness contribution immediately. Continuous intentions expire or transition according to the consumer's declared deadman and disconnect policy. Desired state may persist only when the consumer explicitly declares that behavior.

### A control-producer Resource is not proof of authority

A controller does not prove authority by advertising a control-producer Resource. A malicious peer could advertise the same row. Authority must be bound to cryptographic peer identity through pairing. The consumer must verify that the live stream comes from the authenticated peer that owns the paired producer Resource. The pairing grant authorizes the specific relationship:

```text
(source_peer_id, control_producer_resource_id)
    ->
(target_peer_id, control_consumer_resource_id)
```

The producer Resource provides scoping, compatibility, and lifecycle precision. The peer identity remains the cryptographic principal.

### Reporting and control permissions are independent

Many peers may be allowed to observe an arm while only one peer can control it. Losing or revoking a control session must not interrupt the arm's reporting Resources. Conversely, permission to read telemetry must not imply permission to control the component.

## Authentication And Authorization

An authenticated SDK connection establishes who the remote PeerId is. It does not establish that the peer is allowed to operate anything.

Control is default-deny. Cluster membership permits protocol communication but does not grant actuator authority.

Pairing should:

- Bind authority to stable source and target peer identities
- Bind authority to named producer and consumer Resource IDs
- Be approved through a deliberate trust action, such as a physical action, one-time code, QR flow, or owner signature
- Grant only named Resource relationships, interface versions, and operations
- Authorize the consumer to follow the producer's live intent log
- Authorize the producer to consume the consumer's live outcome Resource
- Support read-state and write-control scopes independently
- Support expiration, revocation, and key rotation
- Avoid unbound static bearer secrets
- Survive controller restart only when the same stable identity is restored

A simple first implementation could store an allow-list of source peer and producer Resource tuples for each consumer Resource and operation scope. A more general design could issue a signed capability grant bound to the complete source and target relationship. Diagnostic read authority for intent and outcome logs remains independently scoped.

Pairing grants durable permission. An active control-session epoch and any lease selected by its arbitration policy convey temporary authority to act from the producer's current live head. These are different concepts.

Local safety systems always dominate remote control. An E-stop must never be remotely cleared through this control surface.

## Control Resource Descriptions

A control-producer descriptor should identify its stable Resource ID and the semantic profiles and schema versions in its intent log. It should also provide the log metadata needed to follow its current live origin. This enables discovery and compatibility matching but grants no authority by itself.

A control-consumer descriptor should describe semantics rather than UI widgets. Buttons, toggles, sliders, and joysticks are optional presentation hints for controller applications.

Conceptually, a consumer descriptor may need:

- Stable endpoint and component identities
- A content-addressed interface definition or schema version
- Supported semantic profiles
- Its outcome Resource and the correlation semantics it provides
- Desired-state properties
- Discrete commands
- Continuous intents
- Content or stream references
- Related feedback Resource references and their semantic roles
- Required authorization scopes
- Arbitration policy
- Disconnect and lease-loss behavior
- Timing, expiry, and scheduling support
- Optional presentation metadata

An outcome Resource descriptor should identify the consumer that publishes it, the control profiles it reports on, and the correlation model that links each outcome to a producer Resource, session epoch, and command ID. Whether one outcome Resource serves all paired producers or each relationship receives its own Resource remains open.

Known semantic profiles should be preferred over an unstructured collection of arbitrary variables. Vendor-specific extensions should remain possible without weakening the common profiles.

## Instruction Classes

The common intent-log model should generalize instruction lifecycle, not pretend every effector has the same behavior.

### Desired state

Append an explicit target state, such as ambient enabled, display brightness, or speaker volume. Desired-state entries should be idempotent.

For example, ambient control should expose `ambient.enabled = true | false`, not a `toggle` instruction. A toggle UI can render this property, but retries must not invert the state accidentally.

### Discrete command

Invoke a bounded operation such as stop playback, clear a display, dock, or reset a recoverable fault.

### Continuous intent

Refresh an intention such as base velocity or live audio by appending new, expiring entries. Safety-sensitive continuous intentions require a validity deadline or deadman policy so loss of the live producer stream produces a known safe result.

A locomotion interface should not rely on forward-button press and release events. A lost release event is dangerous. It should consume an expiring velocity intention that must be refreshed while motion remains desired.

### Content reference

Tell a display or speaker to present content already addressable through an SDK data or streaming mechanism. Large media payloads should not be embedded in intent entries.

### Scheduled instruction

Request an effect at a named-clock time. This could coordinate displays, speakers, lights, and robot behavior against Auki's temporal model. Timing uncertainty and expiry must remain explicit.

## Outcomes Versus Observation

For every processed intent, the control consumer publishes a correlated event to its outcome Resource, such as:

```text
received
authorized
accepted
rejected
```

Longer-running instructions may also report `executing`, `completed`, or `failed`. The event identifies the producer peer and Resource, session epoch, command ID, and sequence number of the intent it answers.

A control producer should follow the consumer's current authenticated live outcome Resource. Profiles that depend on timely acceptance, rejection, or progress may make outcome consumption a requirement rather than a recommendation. Other authorized peers may consume the same Resource or its materialized copies for diagnostics.

An outcome records what the consumer received and decided. `accepted` means the intent passed authorization, arbitration, validation, and current safety gates. Even `completed` is an executor claim, not proof of physical state. Measured
evidence of what happened comes from the consumer's reporting Resources.

The outcome Resource should provide structured rejection reasons, for example:

```text
unauthorized
lease_held
expired
estop_active
not_working_mode
inhibited
invalid_value
unsupported_profile
```



## Arbitration And Disconnect Policy

Different effectors need different policies:

- Locomotion and manipulation will usually require an exclusive lease.
- A display may allow compositing or last-writer-wins behavior.
- A speaker may queue, replace, or mix requests.
- A light may accept shared desired-state writes with explicit precedence.
- A high-level behavior may claim several lower-level components internally.

The control consumer may be paired with more than one producer Resource. Its advertised arbitration policy determines which live producer epochs are currently actionable and how their intentions combine. Under an exclusive policy, intents from other paired producers receive a structured rejection such as `lease_held`; their presence in a valid live log does not bypass arbitration.

The arbitration policy also determines whether the consumer follows every paired live producer and rejects non-selected intents, or follows only producers with currently actionable epochs. The controller must not infer arbitration from Resource type.

Disconnect behavior must also be explicit:

- Velocity should normally decay or stop when its lease or live producer stream expires.
- A display may continue showing its current content.
- A speaker may finish the current asset but reject new work.
- Ambient behavior may either persist or stop, depending on declared policy.



## Initial Vertical Slice

The first target is intentionally small:

```text
controller peer: Ambient Control
producer Resource: ambient-controller
kind: live intent log
profile: ambient_control.v1
controlled peer: Galbot G1
consumer Resource: behavior/ambient
accepts: ambient_control.v1
desired state: ambient.enabled = true | false
outcome Resource: behavior/ambient/outcomes
authorization: producer-to-consumer pairing grant
observation: behavior status plus existing joint and pose Resources
```

The slice should prove:

1. Discovery of compatible producer, consumer, outcome, and reporting Resources
   through the catalog
2. Explicit pairing of producer and consumer peer and Resource identities
3. Default-deny access before pairing
4. Relationship- and operation-scoped authorization after pairing
5. The consumer following only the producer's current authenticated live origin
6. A session epoch with command IDs, ordering, expiry, and replay protection
7. Idempotent desired-state entries
8. Correlated acknowledgement and rejection through the outcome Resource
9. The producer consuming the current live outcome origin while independently
   observing measured state
10. Historical, backfilled, sealed, relayed, cached, and materialized intent and
    outcome copies remaining diagnostic-only
11. Revocation and safe behavior after disconnect or epoch loss
12. No direct command-delivery path alongside the intent log

It should not attempt generic locomotion or arm teleoperation yet.

## Open Questions

- Is `ControlEndpoint`, `EffectorEndpoint`, or another term the public name?
- Is `WorldInterface` useful as a grouping concept for both observations and effects, or does that collapse a boundary better kept explicit?
- Which fields belong to every generalized `ResourceEntry`, and which remain specific to reporting or control variants?
- How does a peer advertise that a consumer Resource is pairable without revealing a protected control descriptor?
- Is the full control descriptor public to cluster members or visible only after pairing?
- Is pairing approved locally by the device, by a Domain owner, or by either?
- Are grants stored by the controlled peer, represented as signed capability tokens, or both?
- How are control schemas content-addressed and version-negotiated?
- What stability and lifecycle rules apply to producer Resource IDs?
- Is an outcome Resource shared across a consumer's pairings or allocated per producer-consumer relationship?
- How are session epochs established, acknowledged, rotated, and invalidated?
- How does reconnect avoid gaps between attaching to a live head and activating an epoch?
- Which retention policies apply to intent and outcome logs?
- Is authenticated origin provided by the direct peer connection, per-entry signatures, or both?
- How are stream admission and diagnostic read scopes represented?
- How does the catalog distinguish an origin stream eligible for actuation from a relayed or materialized diagnostic copy?
- What command and sequence state must survive consumer restart, and how is it
made crash-consistent with non-idempotent physical effects?
- Which arbitration policies belong in the first protocol revision?
- How are controller priority and local autonomy represented?
- How should control status and fault events appear as reporting Resources?
- How should named-clock scheduling express uncertainty and late arrival?
- Which semantic profiles should be standardized first after ambient control?



## Design Stance

The intended separation is:

- The Resource catalog is the single discovery surface for typed network capabilities.
- A control-producer Resource is the live intent log and sole command-delivery path.
- A paired control consumer follows only the producer's current authenticated live origin under an active session epoch.
- Historical, sealed, relayed, cached, backfilled, and materialized intent entries never actuate.
- The consumer's outcome Resource records what it received and decided; the producer should consume its current authenticated live origin.
- Historical, backfilled, sealed, relayed, cached, and materialized outcome copies do not establish current control state.
- Reporting Resources record measured state independently of intentions and outcomes.
- Pairing grants bind a specific producer Resource and peer identity to a specific consumer Resource, interface version, and operation scope.
- Control sessions define epochs, arbitration, and liveness around the paired log-following relationship; they are not a second command transport.
- Every intent is bound to an authenticated origin and carries a control-session
  epoch, stable command ID, monotonic sequence, issue time, and expiry.
  Consumers retain replay-protection state across reconnects and restarts.
- The controlled device remains the final authority over limits and safety.
