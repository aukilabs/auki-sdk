# Effector Control User Stories

## Status

These stories exercise the log-native control model described in `control.md`.
They are behavioral examples, not wire schemas or compatibility commitments.
Resource IDs, profile names, intent fields, settings, and timing values are
illustrative.

Each story distinguishes three records:

- The producer intent log records what the controller requested.
- The consumer outcome Resource records what the controlled peer received and
  decided.
- Reporting Resources record observed controlled-system state independently of
  requests and outcomes. They prove physical effects only when backed by
  appropriate physical sensing.

Whenever a story says an outcome is correlated to a command ID, the complete
correlation includes the producer peer and Resource, control-session epoch,
command ID, and sequence number. The shorter wording avoids repeating that
tuple at every step. Pre-session lifecycle outcomes instead correlate by
producer peer and Resource, request ID, and pairing-scoped lifecycle sequence.
The epoch in `session_opened` is result data, not part of that request key.

Only entries arriving from the paired producer's current authenticated live
origin under an active control-session epoch are eligible for actuation.
Historical, backfilled, sealed, cached, materialized, or otherwise re-served
copies are diagnostic-only. A libp2p circuit relay that preserves the
end-to-end authenticated origin is transport, not Resource re-serving.

## Shared Pairing Sequence

Before any of the three interactions, the controller and controlled Resource
are paired through a deliberate trust action:

1. The controller application discovers the control-consumer Resource and sees
   that pairing is required.
2. Before pairing, it may inspect public capability metadata, but the consumer
   does not activate an actionable epoch for the producer.
3. The person chooses Pair in the controller application.
4. The controlled device or its owner requests the configured trust action,
   such as a physical confirmation, one-time code, QR flow, or owner signature.
5. The approving party confirms the specific producer Resource, consumer
   Resource, interface version, and operation scopes.
6. The controlled peer stores or verifies a grant bound to both authenticated
   peer identities and both Resource IDs.
7. The grant independently authorizes the consumer to follow the producer's
   live intent log and the producer to consume the consumer's live outcome
   Resource.
8. The application reports successful pairing. Pairing alone does not yet
   establish a lease or make an intent actionable.

## Story 1: Change AmbientMovement State And Settings

### User story

As a person responsible for a Galbot G1, I want to turn ambient movement on,
turn it off, and turn it on again with different settings so that I can change
the robot's idle behavior and verify what it actually does.

### Participants and Resources

- Person using an Ambient Control application
- Controller peer: `ambient-control`
- Control-producer Resource: `ambient-controller`
- Controlled peer: `galbot-g1`
- Control-consumer Resource: `behavior/ambient`
- Outcome Resource: `behavior/ambient/outcomes`
- Proposed reporting Resource: `behavior/ambient/status`
- Existing reporting Resource: joint encoders
- Pose currently derived in the UI from the URDF and joint values
- Illustrative profile: `ambient_control.v1`

### Preconditions

- Both peers have joined the same cluster and authenticated each other's peer
  identities.
- The shared pairing sequence has completed for the Resources in this story.
- The robot advertises `behavior/ambient`, its accepted profile, its outcome
  Resource, and its related reporting Resources in the Resource catalog.
- The application advertises `ambient-controller` as a live intent-log
  Resource using `ambient_control.v1`.
- A pairing grant binds:

  ```text
  (ambient-control, ambient-controller)
      ->
  (galbot-g1, behavior/ambient)
  ```

- The grant permits the desired-state properties used in this story.
- This story describes a target supervisor above the current AmbientMovement
  runtime, which launches one bounded 30-300 second run and then returns to
  idle.
- The proposed status Resource initially reports:

  ```text
  desired.enabled = false
  activity = idle
  inhibition_reason = none
  ```

- Local safety systems permit ambient movement.

### Sequence: discover and establish the live control loop

1. The application reads the Resource catalog for `galbot-g1`.
2. It discovers the `behavior/ambient` control consumer.
3. It confirms that the consumer accepts `ambient_control.v1`.
4. It discovers `behavior/ambient/outcomes` from the consumer descriptor.
5. It discovers the proposed behavior-status Resource and the existing joint
   encoder Resource. The UI derives rig pose from the URDF and joint values.
6. The application opens the current authenticated live origin of
   `behavior/ambient/outcomes`.
7. It also opens the reporting Resources needed to render the robot's observed
   state.
8. After the application advertises the current live origin of
   `ambient-controller`, the robot validates the pairing grant and follows that
   live head in non-actuating pre-session mode.
9. The application appends an epoch-less `open_session` request with a stable
   request ID, the next pairing-scoped lifecycle sequence, and the requested
   `ambient_control.v1` profile.
10. The consumer evaluates the request, mints an epoch, and publishes a
    correlated `session_opened` outcome containing the epoch, lease terms, and
    validity. The application consumes that outcome before using the epoch.
11. The application shows ambient movement as off because the proposed
    behavior-status Resource reports `desired.enabled = false` and
    `activity = idle`. It does not infer state merely from the absence of an
    intent.

For the remainder of this story, every intent append uses that authenticated
live origin and carries the active epoch, command ID, sequence, and
class-specific freshness or scheduling semantics unless a step explicitly says
otherwise.

### Sequence: turn AmbientMovement on

12. The person turns the AmbientMovement switch on.
13. The application appends a desired-state intent to `ambient-controller`.
14. The entry contains the active epoch, a stable command ID, the next sequence
    number, and profile version. Its producer origin is authenticated by the
    live stream. The desired state is:

    ```text
    ambient.enabled = true
    ```

15. The robot receives the entry from the paired producer's authenticated live
    origin.
16. The consumer verifies the pairing, epoch, operation scope, command ID,
    sequence, and profile freshness policy.
17. It checks its arbitration policy and local safety state.
18. The outcome Resource records that the intent was received and authorized.
19. The consumer accepts the desired state and records an `accepted` outcome
    correlated to the command ID.
20. The robot's local behavior manager starts ambient movement.
21. The joint encoder Resource reports measured movement, and the UI derives
    rig pose from those joints.
22. `behavior/ambient/status` reports:

    ```text
    desired.enabled = true
    activity = running
    inhibition_reason = none
    ```

23. The consumer records `completed` when its behavior manager confirms the
    transition.
24. The application shows the request as accepted from the outcome Resource.
25. It shows desired and effective activity from the status Resource and
    physical joint movement from the joint encoder Resource.

### Sequence: turn AmbientMovement off

26. The person turns the switch off.
27. The application appends a new desired-state intent with a new command ID,
    the next sequence number, the same active epoch, and:

    ```text
    ambient.enabled = false
    ```

28. The consumer performs the same origin, authorization, ordering, freshness,
    arbitration, and safety checks.
29. The outcome Resource records `received`, `authorized`, and `accepted`.
30. The target supervisor stops scheduling bounded ambient runs and handles any
    current run according to its declared stop policy.
31. The status Resource reports `desired.enabled = false` and
    `activity = idle`.
32. Joint encoders provide independent evidence that movement has ceased,
    subject to any ordinary settling motion. The UI derives the resulting pose.
33. The outcome Resource records `completed`.
34. The application shows the accepted request, observed idle activity, and
    measured joint state.

### Sequence: turn it on again with new settings

35. The person opens the ambient settings.
36. They choose illustrative values such as:

    ```text
    ambient.duration_seconds = 120
    ambient.head_yaw_degrees = 25
    ```

37. The person turns AmbientMovement on again.
38. The application appends one desired-state intent containing both the new
    settings and `ambient.enabled = true`.
39. Combining the properties in one intent avoids briefly enabling the old
    settings before applying the new ones.
40. The entry uses a new command ID and the next sequence number in the active
    epoch.
41. The consumer validates the complete desired state as one unit.
42. If any property is invalid or outside the grant, it rejects the whole
    intent with event `rejected` and a structured reason such as
    `invalid_value`. The application displays the rejection and leaves the
    observed state unchanged.
43. In the successful path, the outcome Resource records `accepted`.
44. The target supervisor applies the new settings and continuously schedules
    bounded runs while enabled and uninhibited.
45. The behavior-status Resource reports `desired.enabled = true`,
    `activity = running`, and the effective settings.
46. Joint encoders report the resulting physical movement, and the UI derives
    rig pose.
47. The outcome Resource records `completed`.
48. The application displays effective activity and measured joints from
    reporting Resources, not merely the values it appended.

### Sequence: retry without toggling accidentally

49. Suppose the application loses its outcome stream after appending the final
    intent and cannot tell whether it was accepted.
50. It reconnects to the current authenticated live outcome origin.
51. It does not replay historical outcome entries as current control state.
52. If the profile permits retry, the application republishes the same desired
    state using the same command ID according to the profile's retry rules.
53. The consumer recognizes an already processed command ID and does not apply
    the transition twice. It publishes a correlated duplicate or current-result
    outcome according to the eventual profile rules.
54. The application reconciles against the current outcome and observed status.
55. Because the operation sets an explicit state rather than toggling, a retry
    cannot accidentally turn ambient movement off.

### Sequence: E-stop, release, and explicit recovery

56. Ambient movement is enabled and the status Resource reports:

    ```text
    desired.enabled = true
    activity = running
    inhibition_reason = none
    ```

57. A local operator activates the robot's E-stop.
58. Local safety immediately stops physical movement without waiting for a new
    control intent.
59. The desired state remains true, because E-stop changes what may execute,
    not what the remote controller last requested.
60. The proposed status Resource reports:

    ```text
    desired.enabled = true
    activity = inhibited
    inhibition_reason = estop
    ```

61. Joint encoders show that movement has stopped. Telemetry remains available
    even though control is inhibited.
62. The local operator releases the E-stop.
63. The consumer does not resume automatically under the profile's
    explicit-resume policy.
64. The status Resource reports:

    ```text
    desired.enabled = true
    activity = inhibited
    inhibition_reason = resume_required
    ```

65. The application displays Resume rather than pretending ambient movement is
    running.
66. The person chooses Resume.
67. The application appends a fresh `desired.enabled = true` intent with a new
    command ID. Reasserting the desired state is the explicit resume signal; no
    special-purpose resume operation is needed.
68. The consumer rechecks local safety, accepts the fresh intent, and publishes
    the correlated outcome.
69. The target supervisor resumes scheduling bounded ambient runs.
70. The status Resource reports:

    ```text
    desired.enabled = true
    activity = running
    inhibition_reason = none
    ```

### Other safety and diagnostic behavior

71. If the pairing grant is revoked, the consumer invalidates the active epoch
    and stops treating new entries as actionable.
72. If the live producer stream is lost, its liveness contribution ends. The
    consumer either preserves or stops ambient movement according to its
    declared disconnect policy, and the reporting Resource exposes the result.
73. A diagnostic peer may materialize the intent and outcome logs.
74. The robot never follows that materialized intent copy for actuation.
75. The materialized logs show what was requested and decided, while the
    reporting Resources show desired state, effective activity, inhibition, and
    available measured joint state.

### End state

- Ambient movement is on with the new settings.
- The intent log contains the requested off/on transitions and setting change.
- The outcome Resource contains correlated consumer decisions.
- The proposed status Resource separates desired, effective, and inhibited
  state, while joint encoders provide existing measured feedback.

## Story 2: Drive A Robot With Keyboard Arrow Keys

### User story

As an operator using a Park-like application, I want to hold and combine the
arrow keys to drive a robot while the application has focus so that motion is
responsive and stops safely when input or connectivity is lost.

### Participants and Resources

- Operator using a Park-like application
- Controller peer: `park`
- Control-producer Resource: `keyboard-drive`
- Controlled peer: `mobile-robot`
- Control-consumer Resource: `locomotion/base`
- Outcome Resource: `locomotion/base/outcomes`
- Reporting Resources for measured base velocity, pose, operating mode, E-stop,
  and faults
- Illustrative profile: `base_velocity_intent.v1`

### Preconditions

- The robot is in a local operating mode that permits remote locomotion.
- The E-stop is clear, but remains locally authoritative.
- The shared pairing sequence has completed for the Resources in this story.
- A pairing grant binds:

  ```text
  (park, keyboard-drive)
      ->
  (mobile-robot, locomotion/base)
  ```
- The grant permits publishing base-velocity intentions.
- The locomotion consumer advertises an exclusive arbitration policy.
- The Park-like application has permission to consume the outcome and reporting
  Resources.
- The measured base velocity is initially zero.

### Sequence: discover and acquire temporary authority

1. The application discovers `locomotion/base` in the Resource catalog.
2. It confirms support for `base_velocity_intent.v1`.
3. It reads the consumer's exclusive arbitration and deadman policies.
4. It discovers `locomotion/base/outcomes` and the related measured-state
   Resources.
5. It advertises `keyboard-drive` as a live intent log.
6. It opens the current authenticated live origin of the outcome Resource and
   the required reporting Resources.
7. The consumer validates the pairing grant and follows `keyboard-drive` from
   its current authenticated live head in non-actuating pre-session mode.
8. The application appends an epoch-less `open_session` request with a stable
   request ID and the next pairing-scoped lifecycle sequence for
   `base_velocity_intent.v1` and an exclusive lease.
9. The consumer's arbitration policy grants the lease, mints an epoch, and
   publishes a correlated `session_opened` outcome containing the epoch, lease
   terms, and validity. The application consumes that outcome before using the
   epoch.
10. Before enabling keyboard input, the application appends an expiring
    zero-velocity intent. It enables driving only after consuming the
    correlated `accepted` outcome and confirming measured velocity remains zero.
    While the driving surface stays armed, its control loop continues refreshing
    the complete target at the declared cadence, including when that target is
    zero.

For the remainder of this story, every velocity append uses that authenticated
live origin and carries the active epoch, command ID, sequence, and validity
duration unless a step explicitly says otherwise.

### Sequence: press and hold the Up arrow

11. The operator focuses the driving surface in the application.
12. They press and hold the Up arrow.
13. The application records `Up = pressed` in local key state.
14. It does not use operating-system key-repeat events as individual movement
    commands.
15. A local control loop converts the complete pressed-key state into an
    illustrative velocity target:

    ```text
    forward_velocity > 0
    yaw_velocity = 0
    ```

16. The application appends a velocity intent with the active epoch, a command
    ID, the next sequence number, and a validity duration. The live stream
    authenticates the producer origin.
17. When the consumer accepts the intent, it starts a consumer-local monotonic
    deadman deadline from that validity duration. The duration is longer than
    the expected refresh interval but short enough for the declared safety
    policy.
18. The consumer verifies the live origin, pairing, epoch, lease, ordering,
    freshness, operating mode, and E-stop.
19. It records a correlated `accepted` outcome.
20. The base controller begins moving forward.
21. The measured-velocity Resource reports acceleration and forward motion.
22. The pose Resource reports the robot's changing position.
23. While Up remains pressed, the application periodically appends fresh
    velocity intents with increasing sequence numbers and renewed validity
    durations. For processed refreshes, the consumer publishes
    `processed_through_sequence` as an inclusive cumulative watermark and
    `latest_accepted_sequence` as the highest accepted sequence at a bounded
    rate. The latter does not imply that intervening sequences were accepted.
    Rejections remain immediate and cause the application to clear its
    pressed-key state and stop issuing nonzero targets.
24. The robot requires fresh accepted intents to continue motion. Its local
    deadman cadence never waits for the application to consume outcomes.

### Sequence: combine Up and Left

25. While still holding Up, the operator presses Left.
26. The local key state becomes `Up = pressed, Left = pressed`.
27. The next intent represents the combined target:

    ```text
    forward_velocity > 0
    yaw_velocity > 0
    ```

28. The consumer accepts the fresh intent under the same epoch and lease and
    sets both `processed_through_sequence` and `latest_accepted_sequence` to
    that sequence.
29. The base controller follows a forward-left arc.
30. Measured velocity and pose Resources show the resulting motion.
31. The application renders those measurements separately from its keyboard
    target.

### Sequence: release keys and stop

32. The operator releases Up while continuing to hold Left.
33. The local key state becomes `Up = released, Left = pressed`.
34. The application immediately appends a fresh target with zero forward
    velocity and nonzero yaw velocity.
35. The consumer accepts the target, advances
    `processed_through_sequence` inclusively, sets
    `latest_accepted_sequence` to that sequence, and transitions to turning in
    place if permitted by the profile.
36. The operator releases Left.
37. The application immediately appends an explicit zero-velocity intent.
38. The outcome Resource advances `processed_through_sequence` inclusively and
    sets `latest_accepted_sequence` to the zero target's sequence.
39. The base controller decelerates according to local safety limits.
40. The measured-velocity Resource reaches zero.
41. The application displays stopped only when measured feedback confirms it.

### Sequence: lose focus or connectivity

42. In a second run, the operator holds Up and the robot is moving.
43. The browser window loses focus.
44. The application clears its local pressed-key state.
45. It attempts to append an explicit zero-velocity intent immediately.
46. The robot does not rely on receiving that final entry.
47. If the zero intent is lost, the consumer-local deadline for the last
    accepted movement intent passes because no refresh arrives.
48. The consumer applies its deadman policy, commands a safe stop, invalidates
    the actuation epoch, and returns to non-actuating pre-session mode.
49. Any delayed or buffered intent from the invalidated epoch is `rejected`
    with `stale_epoch`; receiving one cannot restart motion. Lease loss and
    deadman activation are exposed through the consumer's declared outcome or
    status semantics.
50. The measured-velocity Resource confirms the stop.
51. A network disconnect has the same safety shape: loss of the current live
    producer stream prevents refresh, ends its liveness contribution, and
    requires re-arming under a new epoch.

### Arbitration, rejection, and replay behavior

52. If another paired controller already holds the exclusive lease, the robot
    may observe the keyboard producer but must not actuate from its entries.
53. It records event `rejected` with reason `lease_held` for intents it
    processes under that policy, or declines to activate an actionable epoch
    according to its descriptor.
54. If the E-stop becomes active, local safety overrides the lease.
55. New movement intents receive a structured rejection such as `estop_active`.
56. The reporting Resource for E-stop or operating mode independently reveals
    the local condition.
57. On reconnect, the consumer reattaches to the producer's current live head in
    non-actuating pre-session mode and never backfills missed velocity intents
    for actuation.
58. The application completes a new `open_session` exchange and obtains a new
    epoch.
59. It sends a fresh zero-velocity arming intent and waits for acceptance and
    measured zero velocity before enabling keyboard input.
60. A sealed or materialized copy of the drive log can reconstruct what the
    operator requested but can never move the robot.
61. Previously processed command IDs and sequence numbers remain protected
    against replay across consumer restart.

### End state

- The robot is stopped.
- Motion occurred only while fresh, authorized velocity intentions arrived.
- The outcome Resource records an inclusive cumulative processing watermark,
  the highest accepted sequence, and prompt rejection or safety events.
- Velocity and pose Resources show what the robot physically did.

## Story 3: Play Robot-Local Audio Remotely

### User story

As a remote operator, I want to tell a robot speaker to play an audio file that
already exists on the robot so that playback begins without transferring the
media through the control log.

### Participants and Resources

- Remote operator using a Park-like application
- Controller peer: `park`
- Control-producer Resource: `speaker-controller`
- Controlled peer: `service-robot`
- Control-consumer Resource: `audio/speaker`
- Outcome Resource: `audio/speaker/outcomes`
- Reporting Resource: `audio/speaker/status`
- Robot-local audio asset catalog or equivalent content-addressing mechanism
- Illustrative profile: `audio_playback.v1`

### Preconditions

- The robot has one or more approved audio files on local disk.
- The shared pairing sequence has completed for the Resources in this story.
- The robot exposes stable asset references rather than unrestricted filesystem
  paths.
- The speaker consumer advertises `audio_playback.v1`, its arbitration policy,
  its outcome Resource, and its playback-status Resource.
- A pairing grant binds:

  ```text
  (park, speaker-controller)
      ->
  (service-robot, audio/speaker)
  ```
- The grant permits the relevant playback operations and content namespace.
- Local volume limits and safety policies remain authoritative.

### Sequence: discover available playback capabilities

1. The application discovers `audio/speaker` through the Resource catalog.
2. It confirms that the consumer accepts `audio_playback.v1`.
3. It reads whether the speaker queues, replaces, or mixes overlapping
   requests.
4. It discovers `audio/speaker/outcomes` and `audio/speaker/status`.
5. It discovers or otherwise obtains permitted robot-local asset references.
6. One illustrative entry is:

   ```text
   asset_id = welcome_chime
   content_hash = <hash of the approved local file>
   ```

7. The application advertises `speaker-controller` as a live intent log.
8. It opens the current authenticated live origin of the outcome and
   playback-status Resources.
9. The consumer validates the pairing grant and follows `speaker-controller`
   from its current authenticated live head in non-actuating pre-session mode.
10. The application appends an epoch-less `open_session` request with a stable
    request ID and the next pairing-scoped lifecycle sequence for
    `audio_playback.v1`. The consumer evaluates it, mints an epoch, and
    publishes a correlated `session_opened` outcome containing the epoch, lease
    terms, and validity. The application consumes that outcome before using the
    epoch.

For the remainder of this story, every playback append uses that authenticated
live origin and carries the active epoch, command ID, sequence, and the
profile's freshness or scheduling semantics unless a step explicitly says
otherwise.

### Sequence: trigger the first audio file

11. The operator selects `welcome_chime` and presses Play.
12. The application appends a discrete playback intent containing:

    ```text
    operation = play
    asset_id = welcome_chime
    content_hash = <expected hash>
    ```

13. The entry also contains the active epoch, stable command ID, next sequence
    number, and the profile's freshness policy. If it uses an absolute
    deadline, that deadline identifies its named clock and uncertainty. The
    live stream authenticates the producer origin.
14. The audio bytes are not embedded in the intent log.
15. The consumer verifies the paired live origin, epoch, command ID, ordering,
    freshness, operation scope, and arbitration policy.
16. It resolves `welcome_chime` inside the robot's approved audio namespace.
17. It rejects path traversal, arbitrary absolute paths, and references outside
    the permitted namespace.
18. It verifies that the local file exists and, when required, matches the
    referenced content hash.
19. Before handing the command to the audio subsystem, the consumer durably
    records `execution_reserved` for the command ID. It then records
    `received`, `authorized`, and `accepted` outcomes.
20. The robot hands the reserved command to the audio subsystem, which opens the
    local file and starts playback.
21. The outcome Resource records `executing`.
22. The playback-status Resource reports the audio subsystem's observed
    playback state, selected asset, and progress supported by the profile.
23. The application displays that playback was accepted from the outcome log.
24. It displays that the playback subsystem reports active. Without an acoustic
    reporting Resource, it does not claim that audible sound was physically
    measured.
25. When playback finishes, the audio subsystem reports idle.
26. The outcome Resource records `completed` for the original command ID.

### Sequence: trigger another file while playback is active

27. In a second run, the operator starts a longer robot-local announcement.
28. While it is playing, they select `follow_up_chime` and press Play.
29. The application appends another intent with a new command ID and sequence.
30. The consumer applies its advertised arbitration policy.
31. Under a queue policy, it durably persists the queued command and records
    `accepted`. Queue persistence is not yet an execution reservation.
32. The playback-status Resource reports that the second asset is queued while
    continuing to report the first asset as playing.
33. After the first asset completes and immediately before actuator handoff, the
    consumer durably records `execution_reserved` for `follow_up_chime`.
34. The robot starts `follow_up_chime`, and the outcome Resource records
    `executing` and later `completed` for the second command.
35. Under a replace policy, the consumer would instead durably record
    `execution_reserved` before stopping the first asset or starting the second.
    Both effects are part of the reserved replacement command.
36. Under a reject-while-busy policy, it would publish event `rejected` with an
    illustrative reason such as `busy`, without changing playback.

### Missing, changed, or unauthorized content

37. The operator selects an asset reference that has been removed from disk.
38. The application appends a validly formed playback intent.
39. The consumer authenticates the producer and verifies the pairing, but asset
    validation fails because it cannot resolve the local file.
40. It records `rejected` with an illustrative reason such as
    `content_not_found`.
41. The playback-status Resource remains unchanged.
42. If the asset exists but its hash differs, the consumer rejects it rather
    than playing unexpected content.
43. If the pairing grant does not permit the asset namespace or playback
    operation, the outcome is `rejected` with reason `unauthorized`.
44. None of these failures are inferred from silence; each processed intent has
    a correlated outcome.

### Disconnect and diagnostic behavior

45. If connectivity is lost during playback, the speaker follows its declared
    disconnect policy.
46. A policy may allow the current local asset to finish while refusing new
    work because no fresh producer entries can arrive.
47. A more restrictive profile may stop playback when its lease or epoch ends.
48. The playback-status Resource records the audio subsystem's observed result
    independently.
49. Authorized diagnostic peers may consume or materialize the intent and
    outcome logs to inspect what was requested and decided.
50. If the consumer restarts with `execution_reserved` but no terminal outcome,
    it publishes `unknown_after_restart` and never plays the asset
    automatically. A command ID with a terminal outcome is also never replayed.
51. A robot never follows a materialized or sealed speaker intent log for
    actuation, and replaying history in an analysis tool cannot trigger the
    speaker.

### End state

- The selected robot-local audio has either completed or produced a correlated
  rejection.
- No audio payload travelled through the control intent log.
- The intent and outcome logs preserve the request and consumer decision.
- The playback-status Resource records what the speaker subsystem reported. An
  acoustic sensor would be needed to prove audible physical output.

## Cross-Story Invariants

Across all three stories:

1. Resource discovery does not grant control authority.
2. Pairing binds both peer identities and both Resource identities.
3. The producer intent log is the sole command-delivery path.
4. Before an epoch exists, the paired consumer follows the producer in
   non-actuating pre-session mode.
5. Every actuation intent carries a command ID, epoch, sequence, and
   class-specific freshness or scheduling semantics.
6. The consumer enforces authorization, arbitration, replay protection, and
   local safety before actuation.
7. Desired-state and discrete outcomes are correlated per intent; continuous
   control may use an inclusive cumulative processing watermark plus the
   highest accepted sequence and exact rejection events.
8. The producer should consume the current authenticated live outcome origin.
9. Reporting Resources provide independent observed-state evidence; physical
   effects require appropriate physical sensing.
10. Discrete commands are durably reserved before actuation; ambiguous restarts
    report `unknown_after_restart` and never retry automatically.
11. Historical, materialized, and otherwise re-served copies are useful for
    diagnostics but inert for actuation and current control decisions.
