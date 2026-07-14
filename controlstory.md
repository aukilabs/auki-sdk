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
tuple at every step.

Only entries arriving from the paired producer's current authenticated live
origin under an active control-session epoch are eligible for actuation.
Historical, backfilled, sealed, relayed, cached, or materialized copies are
diagnostic-only.

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
- Reporting Resource: `behavior/ambient/status`
- Existing joint and pose reporting Resources
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
- The robot initially reports `ambient.enabled = false`.
- Local safety systems permit ambient movement.

### Sequence: discover and establish the live control loop

1. The application reads the Resource catalog for `galbot-g1`.
2. It discovers the `behavior/ambient` control consumer.
3. It confirms that the consumer accepts `ambient_control.v1`.
4. It discovers `behavior/ambient/outcomes` from the consumer descriptor.
5. It discovers the behavior-status, joint, and pose Resources referenced as
   measured feedback.
6. The application opens the current authenticated live origin of
   `behavior/ambient/outcomes`.
7. It also opens the reporting Resources needed to render the robot's measured
   state.
8. The robot validates the pairing grant for `ambient-controller`.
9. The robot activates a control-session epoch for the paired relationship and
   communicates the accepted epoch through its eventual epoch-establishment
   mechanism. The application waits until it learns that epoch.
10. After the application advertises the current live origin of
    `ambient-controller`, the robot opens it at its live head without
    requesting historical entries.
11. The application shows ambient movement as off because the measured
    behavior-status Resource reports off. It does not infer state merely from
    the absence of an intent.

For the remainder of this story, every intent append uses that authenticated
live origin and carries the active epoch, command ID, sequence, issue time, and
expiry unless a step explicitly says otherwise.

### Sequence: turn AmbientMovement on

12. The person turns the AmbientMovement switch on.
13. The application appends a desired-state intent to `ambient-controller`.
14. The entry contains the active epoch, a stable command ID, the next sequence
    number, issue time, expiry, and profile version. Its producer origin is
    authenticated by the live stream. The desired state is:

    ```text
    ambient.enabled = true
    ```

15. The robot receives the entry from the paired producer's authenticated live
    origin.
16. The consumer verifies the pairing, epoch, operation scope, command ID,
    sequence, and expiry.
17. It checks its arbitration policy and local safety state.
18. The outcome Resource records that the intent was received and authorized.
19. The consumer accepts the desired state and records an `accepted` outcome
    correlated to the command ID.
20. The robot's local behavior manager starts ambient movement.
21. Joint and pose Resources begin reporting the robot's measured movement.
22. `behavior/ambient/status` reports `ambient.enabled = true`.
23. The consumer records `completed` when its behavior manager confirms the
    transition.
24. The application shows the request as accepted from the outcome Resource.
25. It shows AmbientMovement as actually on from the measured status Resource.

### Sequence: turn AmbientMovement off

26. The person turns the switch off.
27. The application appends a new desired-state intent with a new command ID,
    the next sequence number, the same active epoch, and:

    ```text
    ambient.enabled = false
    ```

28. The consumer performs the same origin, authorization, ordering, expiry,
    arbitration, and safety checks.
29. The outcome Resource records `received`, `authorized`, and `accepted`.
30. The local behavior manager stops initiating ambient movements.
31. The status Resource reports `ambient.enabled = false`.
32. Joint and pose Resources provide independent evidence that the ambient
    behavior has ceased, subject to any ordinary settling motion.
33. The outcome Resource records `completed`.
34. The application shows both the accepted request and the measured off state.

### Sequence: turn it on again with new settings

35. The person opens the ambient settings.
36. They choose illustrative values such as:

    ```text
    ambient.intensity = 0.35
    ambient.pause_interval_s = 8
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
    measured state unchanged.
43. In the successful path, the outcome Resource records `accepted`.
44. The behavior manager applies the new settings and enables ambient movement.
45. The behavior-status Resource reports the enabled state and the effective
    settings, if that profile exposes them as measured state.
46. Joint and pose Resources reflect the resulting physical movement.
47. The outcome Resource records `completed`.
48. The application displays the effective state from reporting Resources, not
    merely the values it appended.

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
54. The application reconciles against the current outcome and measured status.
55. Because the operation sets an explicit state rather than toggling, a retry
    cannot accidentally turn ambient movement off.

### Safety and diagnostic behavior

56. If the pairing grant is revoked, the consumer invalidates the active epoch
    and stops treating new entries as actionable.
57. If the live producer stream is lost, its liveness contribution ends. The
    consumer either preserves or stops ambient movement according to its
    declared disconnect policy, and the reporting Resource exposes the result.
58. A diagnostic peer may materialize the intent and outcome logs.
59. The robot never follows that materialized intent copy for actuation.
60. The materialized logs show what was requested and decided, while the
    reporting Resources remain the evidence of what the robot physically did.

### End state

- Ambient movement is on with the new settings.
- The intent log contains the requested off/on transitions and setting change.
- The outcome Resource contains correlated consumer decisions.
- Reporting Resources contain the independently measured robot state.

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
7. The consumer validates the pairing grant.
8. Its arbitration policy grants the producer a control lease and activates a
   control-session epoch. It communicates the accepted epoch through its
   eventual epoch-establishment mechanism, and the application waits until it
   learns that epoch.
9. The consumer follows `keyboard-drive` from its current authenticated live
   head.
10. Before enabling keyboard input, the application appends an expiring
    zero-velocity intent. It enables driving only after consuming the
    correlated `accepted` outcome and confirming measured velocity remains zero.

For the remainder of this story, every velocity append uses that authenticated
live origin and carries the active epoch, command ID, sequence, issue time, and
expiry unless a step explicitly says otherwise.

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

16. The application appends an expiring velocity intent with the active epoch,
    a command ID, the next sequence number, issue time, and expiry. The live
    stream authenticates the producer origin.
17. The expiry is later than the expected refresh interval but short enough for
    the consumer's declared deadman policy.
18. The consumer verifies the live origin, pairing, epoch, lease, ordering,
    expiry, operating mode, and E-stop.
19. It records a correlated `accepted` outcome.
20. The base controller begins moving forward.
21. The measured-velocity Resource reports acceleration and forward motion.
22. The pose Resource reports the robot's changing position.
23. While Up remains pressed, the application periodically appends fresh
    velocity intents with increasing sequence numbers and new expiries. For
    every refresh it processes, the consumer publishes a correlated outcome
    that the application consumes. A rejection causes the application to clear
    its pressed-key state and stop issuing nonzero targets.
24. The robot requires those refreshes to continue motion.

### Sequence: combine Up and Left

25. While still holding Up, the operator presses Left.
26. The local key state becomes `Up = pressed, Left = pressed`.
27. The next intent represents the combined target:

    ```text
    forward_velocity > 0
    yaw_velocity > 0
    ```

28. The consumer accepts the fresh intent under the same epoch and lease and
    publishes a correlated outcome consumed by the application.
29. The base controller follows a forward-left arc.
30. Measured velocity and pose Resources show the resulting motion.
31. The application renders those measurements separately from its keyboard
    target.

### Sequence: release keys and stop

32. The operator releases Up while continuing to hold Left.
33. The local key state becomes `Up = released, Left = pressed`.
34. The application immediately appends a fresh target with zero forward
    velocity and nonzero yaw velocity.
35. The robot publishes the correlated outcome and transitions to turning in
    place if permitted by the profile.
36. The operator releases Left.
37. The application immediately appends an explicit zero-velocity intent.
38. The outcome Resource records acceptance.
39. The base controller decelerates according to local safety limits.
40. The measured-velocity Resource reaches zero.
41. The application displays stopped only when measured feedback confirms it.

### Sequence: lose focus or connectivity

42. In a second run, the operator holds Up and the robot is moving.
43. The browser window loses focus.
44. The application clears its local pressed-key state.
45. It attempts to append an explicit zero-velocity intent immediately.
46. The robot does not rely on receiving that final entry.
47. If the zero intent is lost, the last movement intent reaches its expiry
    because no refresh arrives.
48. The consumer applies its deadman policy and commands a safe stop.
49. If a late intent is processed, the outcome is `rejected` with `expired`.
    Lease loss and deadman activation are exposed through the consumer's
    declared outcome or status semantics.
50. The measured-velocity Resource confirms the stop.
51. A network disconnect has the same safety shape: loss of the current live
    producer stream prevents refresh and ends its liveness contribution.

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
57. On reconnect, the consumer follows only the producer's new current live
    head under an accepted epoch.
58. It never backfills missed velocity intentions for actuation.
59. A sealed or materialized copy of the drive log can reconstruct what the
    operator requested but can never move the robot.
60. Previously processed command IDs and sequence numbers remain protected
    against replay across consumer restart.

### End state

- The robot is stopped.
- Motion occurred only while fresh, authorized velocity intentions arrived.
- The outcome Resource records consumer decisions for each processed intent.
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
9. The consumer validates the pairing grant and activates a control-session
   epoch. It communicates the accepted epoch through its eventual
   epoch-establishment mechanism, and the application waits until it learns
   that epoch.
10. The consumer follows `speaker-controller` from its current authenticated
    live head.

For the remainder of this story, every playback append uses that authenticated
live origin and carries the active epoch, command ID, sequence, issue time, and
expiry unless a step explicitly says otherwise.

### Sequence: trigger the first audio file

11. The operator selects `welcome_chime` and presses Play.
12. The application appends a discrete playback intent containing:

    ```text
    operation = play
    asset_id = welcome_chime
    content_hash = <expected hash>
    ```

13. The entry also contains the active epoch, stable command ID, next sequence
    number, issue time, and expiry. The live stream authenticates the producer
    origin.
14. The audio bytes are not embedded in the intent log.
15. The consumer verifies the paired live origin, epoch, command ID, ordering,
    expiry, operation scope, and arbitration policy.
16. It resolves `welcome_chime` inside the robot's approved audio namespace.
17. It rejects path traversal, arbitrary absolute paths, and references outside
    the permitted namespace.
18. It verifies that the local file exists and, when required, matches the
    referenced content hash.
19. The outcome Resource records `received`, `authorized`, and `accepted`.
20. The robot opens the local file and starts its audio subsystem.
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
31. Under a queue policy, it records `accepted` for the intent.
32. The playback-status Resource reports that the second asset is queued while
    continuing to report the first asset as playing.
33. After the first asset completes, the robot starts `follow_up_chime`.
34. The outcome Resource records `executing` and later `completed` for the
    second command.
35. Under a replace policy, the consumer would instead stop the first asset
    according to local rules before starting the second.
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
48. The playback-status Resource records the measured result independently.
49. Authorized diagnostic peers may consume or materialize the intent and
    outcome logs to inspect what was requested and decided.
50. A duplicate command ID does not play the asset twice, including after
    consumer restart.
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
4. The consumer follows only the paired producer's current authenticated live
   origin under an active epoch.
5. Every intent carries a command ID, epoch, sequence, issue time, and expiry.
6. The consumer enforces authorization, arbitration, replay protection, and
   local safety before actuation.
7. The consumer publishes correlated decisions to an outcome Resource.
8. The producer should consume the current authenticated live outcome origin.
9. Reporting Resources provide independent observed-state evidence; physical
   effects require appropriate physical sensing.
10. Historical and materialized copies are useful for diagnostics but inert for
    actuation and current control decisions.
