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
    |-- proposed status Resource: behavior/ambient/status
    `-- existing feedback: joint encoders; pose derived by the UI
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

A Resource owned by a controlled peer that identifies an executor of authorized
intentions. It describes the semantic profiles it accepts, its arbitration and
disconnect policies, the outcome Resource it publishes, and related reporting
Resources with their observed-state roles.

The consumer follows the current live origin of a paired producer Resource. It
never actuates from a historical, materialized, or otherwise re-served copy. A
control consumer is also referred to as a control endpoint in this note.

### Outcome Resource

A live log published by the controlled peer with events correlated to producer intent entries. It records whether the consumer received, authorized, accepted, rejected, is executing, completed, or failed an intention.

An outcome Resource reports the consumer's protocol and execution decisions, not measured physical state. A control producer should consume the outcome Resource referenced by the consumer it controls.

### Controller

An SDK peer that owns a control-producer Resource, obtains a pairing grant for a
compatible control-consumer Resource, and consumes the consumer's outcome and
observed-state Resources.

### Pairing grant

The authorization that binds a named control-producer Resource on an authenticated controller peer to a named control-consumer Resource and selected operations on an authenticated controlled peer.

### Control session

A live, paired following relationship between a control-producer Resource and a control-consumer Resource under a named session epoch. The consumer follows the producer's live intent-log head, while the producer follows the current authenticated live origin of the consumer's outcome Resource. The consumer's arbitration policy determines whether the relationship is exclusive, shared, queued, mixed, composited, or last-writer-wins.

A control session is an authorization, arbitration, epoch, and liveness boundary. It is not an alternative command transport outside the Resource streams.

The control-session epoch is distinct from the SDK recording Session and its Session ID.

Before a control session exists, a paired consumer follows the producer's live
head in a non-actuating pre-session mode. This is how session lifecycle entries
reach the consumer without granting actuator authority.

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

The Resource catalog remains the single discovery surface, and Resource streams
carry both intentions and outcomes. The consumer opens and follows the paired
producer's intent log. The producer opens and should follow the outcome Resource
referenced by the consumer from its current authenticated origin. Reporting
Resources independently carry observations of controlled-system state.

The intent and outcome logs are both transport and record. Diagnostics consume the same records used by the control loop; there is no optional mirror that can diverge from a separate direct command path.

### Session bootstrap

Pairing authorizes a consumer to follow a producer, but does not establish an
actionable epoch. Session bootstrap uses the same intent and outcome logs:

1. The paired consumer follows the producer's current authenticated live head
   in non-actuating pre-session mode.
2. The producer appends an epoch-less `open_session` lifecycle request with a
   stable request ID, a monotonic pairing-scoped lifecycle sequence, requested
   profile, and requested lease or arbitration terms.
3. The consumer verifies the pairing grant, origin, requested scope, local
   safety state, and arbitration availability.
4. If accepted, the consumer mints an epoch and publishes a `session_opened`
   outcome correlated to the request. The outcome includes the epoch, granted
   lease terms, and validity.
5. The producer consumes `session_opened` before appending any actuation intent
   for that epoch.

Pre-session mode recognizes session lifecycle entries but never actuates.
Actuation intents without an accepted epoch are rejected with a structured
reason such as `session_required`. Rejected session requests receive a
correlated `rejected` outcome. The consumer retains enough lifecycle request
state to reject duplicate or stale session requests.

### Live-origin actuation boundary

The existence of an intent entry in a log is not sufficient authority to actuate. A control consumer may act only on an entry that satisfies all of these conditions:

1) The producer peer and Resource are the exact source tuple named by an active pairing grant.

2) The entry arrives while following that producer Resource's current live origin directly from its authenticated owner.

3) The actuation entry belongs to the session epoch currently accepted by the
consumer's arbitration policy.

4) Its command ID has not been processed, its sequence is strictly greater than
the last accepted sequence in that epoch, and its class-specific freshness
policy is satisfied.

5) The requested operation and interface version are within the pairing grant.

Historical reads and backfill are never eligible for actuation. Neither are
entries read from sealed, cached, materialized, or otherwise re-served copies
held by a peer other than the authenticated origin, even when those copies
preserve the original `source_peer_id`. Such entries remain useful for
diagnostics and audit.

The live-origin rule separates availability from authority. Authorized
diagnostic peers may read and materialize intent and outcome logs, but only the
paired consumer following the authenticated origin under its active epoch may
interpret new entries as actionable. An end-to-end authenticated connection
that traverses a libp2p circuit relay still comes from the origin; transport
relay is not Resource re-serving.

The producer should likewise consume outcomes from the consumer's current
authenticated live origin when making current control decisions. Historical,
backfilled, sealed, cached, materialized, or otherwise re-served outcomes
remain valid diagnostic records, but do not establish the consumer's current
control state.

### Epochs, ordering, and replay protection

Every actuation intent needs:

- A stable command ID
- A session epoch
- A sequence number monotonic within that epoch
- Freshness or scheduling semantics appropriate to its instruction class
- An authenticated producer origin established by the live transport,
  per-entry authentication, or both

The consumer must retain enough command and sequence state across reconnects
and restarts to reject duplicates. Reopening a stream does not make earlier
entries actionable, and a new epoch does not revive commands from an old one.
The exact lifecycle wire shapes and durable storage mechanisms remain protocol
design questions.

Desired-state entries should be idempotent. Continuous intents are expiring,
latest-value inputs. Discrete commands use at-most-once delivery at the consumer
boundary: before handing a command to the actuator, the consumer durably records
`execution_reserved` or `committed_for_execution`.

If the consumer restarts after that reservation but before recording a terminal
outcome, it publishes `unknown_after_restart` and never retries the physical
effect automatically. This avoids duplicate actuation but does not claim
whether the effect occurred. Stronger guarantees require an idempotent actuator
that participates using the same command ID.

Loss of the live origin stream ends its liveness contribution immediately. Continuous intentions expire or transition according to the consumer's declared deadman and disconnect policy. Desired state may persist only when the consumer explicitly declares that behavior.

### Timing and freshness

Safety-sensitive continuous control uses consumer-local monotonic elapsed time,
not direct comparison with a remote wall clock. On each accepted fresh intent,
the consumer starts or refreshes a local deadman deadline from the intent's
declared validity duration. If the deadline passes, the consumer applies its
safe transition even if the producer has not consumed an outcome.

A delayed same-epoch entry must not restart motion after that transition. A
safety-sensitive profile therefore declares a post-deadman re-arm rule. The
locomotion profile should invalidate the actuation epoch, return to
non-actuating pre-session mode, and reattach at the producer's current live head
without backfill. Motion then requires a new `open_session` exchange and an
accepted zero-velocity arming intent under the new epoch. Buffered entries from
the old epoch remain inert.

Issue timestamps may be included for diagnostics, but are not a safety clock by
themselves. Absolute expiry or scheduled execution uses Auki's named-clock
model: the timestamp identifies its `clock_id` and `clock_hash`, conversion
uncertainty remains explicit, and the profile declares late-arrival behavior.

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

An outcome Resource descriptor should identify the consumer that publishes it,
the control profiles it reports on, and its correlation model. Actuation
outcomes correlate to producer Resource, session epoch, command ID, and
sequence. Pre-session lifecycle outcomes correlate to producer Resource,
request ID, and lifecycle sequence; `session_opened` carries the newly minted
epoch as result data. Whether one outcome Resource serves all paired producers
or each relationship receives its own Resource remains open.

Known semantic profiles should be preferred over an unstructured collection of arbitrary variables. Vendor-specific extensions should remain possible without weakening the common profiles.

## Instruction Classes

The common intent-log model should generalize instruction lifecycle, not pretend every effector has the same behavior.

### Desired state

Append an explicit target state, such as ambient enabled, display brightness, or speaker volume. Desired-state entries should be idempotent.

For example, ambient control should expose `ambient.enabled = true | false`, not a `toggle` instruction. A toggle UI can render this property, but retries must not invert the state accidentally.

Desired state is not effective activity. An ambient profile should report these
separately:

```text
desired.enabled
activity = idle | running | inhibited | faulted
inhibition_reason
resume_policy
```

An E-stop or other local safety gate may stop physical movement while
`desired.enabled` remains true. Releasing the E-stop does not imply automatic
restart under an explicit-resume policy. A fresh
`desired.enabled = true` intent with a new command ID may reassert consent and
resume the behavior without introducing a special-purpose resume operation.

### Discrete command

Invoke a bounded operation such as stop playback, clear a display, dock, or reset a recoverable fault.

### Continuous intent

Refresh an intention such as base velocity or live audio by appending new, expiring entries. Safety-sensitive continuous intentions require a validity deadline or deadman policy so loss of the live producer stream produces a known safe result.

A locomotion interface should not rely on forward-button press and release events. A lost release event is dangerous. It should consume an expiring velocity intention that must be refreshed while motion remains desired.

### Content reference

Tell a display or speaker to present content already addressable through an SDK data or streaming mechanism. Large media payloads should not be embedded in intent entries.

### Scheduled instruction

Request an effect at a named-clock time. This could coordinate displays,
speakers, lights, and robot behavior against Auki's temporal model. The request
identifies `clock_id`, `clock_hash`, timestamp, expiry, and late-arrival policy.
Conversion uncertainty remains explicit.

## Outcomes Versus Observation

For desired-state and discrete intents, the control consumer publishes a
correlated event to its outcome Resource, such as:

```text
received
authorized
accepted
rejected
```

Longer-running instructions may also report `executing`, `completed`, or `failed`. The event identifies the producer peer and Resource, session epoch, command ID, and sequence number of the intent it answers.

For high-rate continuous intents, the consumer may publish cumulative
outcomes at a bounded rate instead of one durable event per refresh.
`processed_through_sequence` is the inclusive highest sequence the consumer has
processed regardless of disposition. `latest_accepted_sequence` is only the
highest accepted sequence; it does not imply that every intervening sequence
was accepted. Rejections identify their exact sequence, while lease loss and
safety transitions remain prompt. Producer outcome consumption never paces the
consumer's deadman or actuation loop.

A control producer should follow the consumer's current authenticated live outcome Resource. Profiles that depend on timely acceptance, rejection, or progress may make outcome consumption a requirement rather than a recommendation. Other authorized peers may consume the same Resource or its materialized copies for diagnostics.

An outcome records what the consumer received and decided. `accepted` means the
intent passed authorization, arbitration, validation, and current safety gates.
Even `completed` is an executor claim, not proof of physical state. Reporting
Resources provide independent observed-state evidence; proof of a physical
effect requires appropriate physical sensing.

The outcome Resource should provide structured rejection reasons, for example:

```text
unauthorized
lease_held
expired
stale_epoch
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

The consumer follows every paired producer's current authenticated live head so
that any producer can request a session. A producer without an actionable epoch
is followed in lifecycle-only, non-actuating mode. Arbitration determines which
established epochs may actuate and whether non-selected actuation intents are
rejected or ignored; it never prevents session lifecycle entries from reaching
the consumer. The controller must not infer arbitration from Resource type.

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
proposed status: behavior/ambient/status
existing observation: joint encoders; pose currently derived by the UI
```

The current AmbientMovement runtime launches one bounded 30-300 second run and
then returns to idle. The vertical slice proposes a supervisor above that
runtime: while `desired.enabled` is true and no safety condition inhibits it,
the supervisor continuously schedules bounded runs using the effective
settings. `desired.enabled = false` stops scheduling and handles any current run
according to the declared stop policy.

The slice should prove:

1. Discovery of compatible producer, consumer, outcome, and reporting Resources
   through the catalog
2. Explicit pairing of producer and consumer peer and Resource identities
3. Default-deny access before pairing
4. Relationship- and operation-scoped authorization after pairing
5. The consumer following only the producer's current authenticated live origin
6. A session epoch with command IDs, ordering, class-specific freshness, and
   replay protection
7. Idempotent desired-state entries
8. Correlated acknowledgement and rejection through the outcome Resource
9. The producer consuming the current live outcome origin while independently
   observing measured state
10. Historical, backfilled, sealed, cached, materialized, and otherwise
    re-served intent and outcome copies remaining diagnostic-only
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
- What exact wire shapes represent `open_session`, `session_opened`, epoch
  rotation, and session closure?
- How does reconnect avoid gaps between attaching to a live head and activating an epoch?
- Which retention policies apply to intent and outcome logs?
- Is authenticated origin provided by the direct peer connection, per-entry signatures, or both?
- How are stream admission and diagnostic read scopes represented?
- How does the catalog distinguish an origin stream eligible for actuation from
  a materialized or otherwise re-served diagnostic copy?
- What durable store and actuator integrations implement
  `execution_reserved`, terminal outcomes, and `unknown_after_restart`?
- Which arbitration policies belong in the first protocol revision?
- How are controller priority and local autonomy represented?
- How should control status and fault events appear as reporting Resources?
- How should named-clock scheduling express uncertainty and late arrival?
- Which semantic profiles should be standardized first after ambient control?



## Design Stance

The intended separation is:

- The Resource catalog is the single discovery surface for typed network capabilities.
- A control-producer Resource is the live intent log and sole command-delivery path.
- A paired control consumer actuates only from the producer's current
  authenticated live origin under an active session epoch.
- Before an epoch exists, a paired consumer follows in non-actuating pre-session
  mode and accepts only session lifecycle entries.
- `open_session` is epoch-less; `session_opened` returns the consumer-minted
  epoch, lease terms, and validity through the outcome Resource.
- Historical, sealed, cached, backfilled, materialized, and otherwise re-served
  intent entries never actuate.
- The consumer's outcome Resource records what it received and decided; the producer should consume its current authenticated live origin.
- Historical, backfilled, sealed, cached, materialized, and otherwise re-served
  outcome copies do not establish current control state.
- Reporting Resources record observed state independently of intentions and
  outcomes; physical claims require appropriate sensing.
- Pairing grants bind a specific producer Resource and peer identity to a specific consumer Resource, interface version, and operation scope.
- Control sessions define epochs, arbitration, and liveness around the paired log-following relationship; they are not a second command transport.
- Every actuation intent is bound to an authenticated origin and carries a
  control-session epoch, stable command ID, monotonic sequence, and
  class-specific freshness or scheduling semantics.
- Discrete commands are reserved durably before actuation; ambiguous restarts
  produce `unknown_after_restart` and never automatic physical retry.
- The controlled device remains the final authority over limits and safety.
