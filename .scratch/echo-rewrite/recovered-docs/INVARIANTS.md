# Product Invariants

Product law for this codebase. Read the relevant section before touching that system, and
say in the gate's G1 section which invariants you relied on.

These were prose rules under `.claude/rules/` until 2026-08-28. They moved here because
injecting them into every session is not what made them hold — encoding them as tests is.
Where an invariant is expressible as a test, **the test is the authority and this file is
the explanation**. Where it is not, this file is the authority.

Status legend: **[T]** covered by a breaking test · **[D]** documented only, not yet
encodable · **[T?]** nominated for a test, not yet written.

---

## 1. Aim — product law (settled with the user 2026-08-28)

This section **replaces** the pre-v64 aim policy. Two earlier decisions were formally
reversed and must not be reintroduced without a new user decision:

- The v44 "mid-spray assist rejected, never offer again" ruling is **void**.
- The v63 "target body is a force-free zone, zero output inside" law is **dead**.

**Architecture: three separate lanes.** Aim assist, a standalone RCS, and Trigger are
composable features, not one merged controller. Consequence: `SprayRecoilState` is not to be
deleted — it is the basis of the standalone RCS feature.

| # | Invariant | |
| --- | --- | --- |
| A1 | Assist during firing is permitted. Target steering is not gated on shot count. | **[T]** `recoil.rs` phase-predicate test, plus `worker/session/tests.rs` `target_controller_runs_in_every_phase_except_an_explicit_barrier` which fails if a phase branch reappears in the dispatch |
| A2 | The hand always wins. The aim lane yields **proportionally and continuously** to opposing physical motion — no latch, no quiet window, no fixed recovery ramp. The RCS lane does **not** yield to opposition, but it **supports** the hand rather than adding to it: the player's own pull along the recoil pays the booked compensation first and the lane pays the remainder (decided 2026-08-29: the player always pulls; the lock is there to help). A pitch-dominant pull down the recoil during a burst is that support, never a takeover: it cannot grade as opposition or latch a yield, whichever way the tracking correction points that tick. | **[T]** decided 2026-08-29 (user, option B): "no latch" is absolute. `aim-v84` removed the total-cancellation latch end to end — the 80 ms quiet window, the 150 ms recovery ramp, the `Yielding` state, the commit-time consumption, and the player-yield rebase are deleted; a total cancellation is one withheld tick per tick, on and off the body alike, and authority returns the tick the hand stops opposing (`lock/tests/player_authority.rs` `total_opposition_withholds_the_tick_and_keeps_the_pull`, `driver/tests/lock_output.rs` `sustained_opposition_keeps_withholding_and_the_owner`). The `aim-v83` late-gate closure stands as history: the gate commits nothing about the hand. The RCS half is WP-3, unstarted — except that hand motion can no longer discard booked compensation, which this row's own text requires. |
| A3 | Inside the target body, tracking continues gently (high smoothing, reduced force, never zero) so a strafing target stays tracked. No hard on/off boundary at the capsule surface — entering and leaving must not stutter. A hand adjustment on the body is weighed locally (A2's continuous weight), never latched, never a pull boundary. | **[T]** implemented 2026-08-29 (`aim-v71`, hand weighing `aim-v72`: `worker/session/tests.rs` `body_containment_weighs_the_hand_locally_without_a_rebase`, `lock/tests/player_authority.rs` `body_contact_overrides_conflict_barrier_and_off_body_lock_resumes`): body contact keeps the pull and its carry and eases engagement to a 35% floor over 60 ms, leaving eases back; `lock/tests/binding.rs` `real_body_contact_keeps_tracking_at_a_reduced_never_zero_share`, `lock/tests/momentum.rs` `body_contact_and_exit_are_one_continuous_pull`, `lock/tests/pull_identity.rs` `body_contact_keeps_the_pull_and_its_generation`, `driver/tests/lock_output.rs` `a_new_press_starts_without_carry_even_after_body_contact`. Live contract: pull episodes per minute and the `body-contact` retire cause must collapse against capture 011417 (197/min, 70 body-contact). Since `aim-v83` a late suffix racing a body-entry proposal is no longer a pull boundary by itself: the `late-body-conflict` retirement (`retire_target_motion_preserving_owner`) and its rebase observer tick are removed end to end — `driver/tests/lock_output.rs` `a_rejected_body_entry_proposal_leaves_committed_pull_and_carry_untouched`; the trace label stays append-only and is never emitted. Since `aim-v84` the committed-state exemption itself is gone: weight-0 is weighed in the transition against this tick's fresh geometry on both sides of the capsule surface. |
| A2b | One recoil planner, never two on a tick. With Aim locked it runs inside the aim lane as feedforward — each shot edge's punch change is booked at once and paid out across ~80% of the shot interval (never in one packet), the controller is shown the ray as it will be once the whole debt has landed, and it chases only the target; with Aim off it runs standalone. A rejected transaction restores the planner's basis and unpaid debt rather than losing the change. The hand's pull along the recoil pays the debt first (a pull made before the next edge is held as bounded credit against it, per burst), and a burst's first kick is booked against the pre-burst deflection the lane kept watching, not against zero. | **[D]** measured 2026-08-28: with the hand relaxed the aim lane alone leaves 0.26 deg of spray miss, matching the 0.27 deg measured on landed shots. Composing a second lane on top double-compensates. Enforced 2026-08-28: one lane owns each tick (`TickLane`), and an Aim-owned tick retires the recoil basis it did not observe — `an_aim_owned_tick_retires_the_recoil_basis_it_did_not_observe`. |
| A4 | The aim lane's motion profile is the jerk-limited S-curve (accelerate, drag, brake). | **[T]** `controller.rs` profile tests |
| A5 | The bound owner is held until it dies, visibility is lost past grace, or the key is released. No automatic retarget mid-hold and no automatic spray transfer. After the owner dies, re-acquisition starts from zero and the player's hand leads the swing. A burst end (shots back to zero) is none of those: it resets spray state only, and a checked-zero observer tick keeps the pull. | **[T]** `echo/engine/tests/owner_exclusivity.rs`; burst end since `aim-v75`: `worker/session/tests.rs` `post_shot_phase_retains_owner_and_burst_end_keeps_it_too`, `lock/tests/pull_identity.rs` `a_checked_observer_keeps_the_pull_for_the_next_steering_tick`. A rejected serialized transaction is none of those either (`aim-v83`): the committed owner, pull, generation, carry and correction reference survive a `MotionSequenceChanged` reject. A total hand cancellation is also none of those (`aim-v84`): it withholds that tick's output only; the pull, owner, carry and generation continue. The pull ends solely on owner death, visibility lost past grace, key release, or session end. |
| A6 | Left Mouse is a legitimate Aim activation key. Trigger still refuses it — Trigger must not steal the manual fire button. | **[T]** keybind tests |

Supporting lifecycle invariants, unchanged by the v64 reversal:

- **[T]** The Aim acquisition radius (`AimFovHalfAngle`) is capped at 5 degrees, in half-degree
  steps from 0.5 (decided 2026-09-05, user). The menu slider offers nothing wider and a saved
  value above the cap clamps down on load (`overlay/tests/aim_controls.rs`
  `slider_card_is_the_complete_drag_hit_area`, `config/tests/versions.rs`
  `v24_settings_keep_every_live_preference_and_drop_the_retired_aim_point_switch`).
- **[D]** Aim, Trigger, and Aim's FOV guide require a supported firearm.
- **[D]** The first scoped Aim output demand wakes its producer immediately and stays
  fail-closed until a fresh Raw Input registration self-check acknowledges the same demand
  generation.
- **[T]** A Raw Input press transition counted between two ticks is release proof for both
  the idle sentinel's latch and the session episode's latch (`worker/episode/tests.rs`
  `a_counted_press_transition_is_release_proof_for_the_latch`); a required release keeps the
  warm resource bundle while every hard authority still holds. A session that ends on soft
  context (the menu, focus, a withheld output ticket) retains its bundle into the warm store
  and the store's grace decides; only hard authorities refuse retention at exit
  (`worker/warm_resources.rs` `only_hard_authorities_refuse_retention_at_exit`,
  `a_session_exit_leaves_the_soft_context_to_the_warm_store_grace`).
- **[D]** Aim may hold an event-driven Raw Input receiver while inactive so short physical
  transitions are not lost; standby retains no motion work and no output authority.
- **[D]** The last Aim output lease revokes readiness synchronously before returning to
  standby.
- **[D]** A static Aim-only surface is redrawn only when its presentation inputs or
  visibility lifecycle change.

## 2. Trigger

- **[D]** Contact-only Trigger and wallbang acquisition are preserved unless the user
  explicitly approves a broader policy. Timing, freshness, or diagnostic work must never
  silently widen the eligible target set.
- **[D]** Trigger states blocked by Nade output ownership or Aim priority use a bounded
  inactive lane and must not keep the active contact cadence.
- **[T]** Trigger is Hold-only (decided 2026-09-05, user). There is no Always On mode: the
  feature fires only while its bound key is physically held, `TriggerSettings` carries no
  activation mode, the menu offers none, and a saved `always_on` decodes to Hold
  (`config/tests/versions.rs` `always_on_is_retired_for_aim_and_trigger_at_every_schema_version`,
  `triggerbot/tests/policy.rs` `aim_priority_matches_the_hold_key_matrix`).

## 3. Nade

- **[D]** A supported grenade is required before feature work becomes eligible.
- **[D]** Without a supported held grenade, only a bounded item and liveness sentinel runs.
  Map, catalog, player, pose, calibration, Raw Input, projection, and drawing stay suspended.
- **[D]** Current map and item coverage is proven before detailed player, pose, calibration,
  projection, or marker work. Known-missing coverage keeps only the bounded recovery sentinel.
- **[D]** Nade owns Raw Input demand from its final eligibility and control path.
- **[D]** A live alignment session retains Raw Input demand regardless of presentation status
  until its generated-output lease is released.
- **[D]** Pending Nade Raw Input demand must not arm output.
- **[D]** Every early or terminal publication reconciles Raw Input demand ownership in the
  same iteration.
- **[D]** Repeated unchanged Nade state reuses immutable map, label, and marker ownership.

## 4. ESP, Bomb Timer, Footsteps

- **[T]** ESP paces itself without reading another feature's state
  (`esp_paces_itself_without_reading_another_features_state`).
- **[D]** ESP, Bomb Timer, and Footsteps must not inherit the Nade, Aim, Trigger, or Aim-FOV
  item gates.
- **[D]** ESP applies its configured inclusive display-distance gate to the compact identity,
  vitals, and origin sample *before* head or name reads and before Skeleton or Weapon proof
  construction.
- **[D]** An empty ESP discovery scene must not run the full-rate presentation lane, and must
  not retain model or equipment catalogs beyond a bounded anti-thrash grace.
- **[D]** Bomb Timer must not inherit a local-player anchor it does not consume — without
  weakening the team and origin proof that enemy and Footstep scans require.
- **[D]** Footstep ground and static-visibility work requires an in-range candidate, a live
  marker, or a documented bounded anti-thrash grace.
- **[D]** With no planted objective, objective-only world and presentation work stays on the
  semantic low-rate lane.

## 5. Raw Input authority

- **[D]** A single asynchronous Up observation is not proof of a lost release.
- **[D]** Treating a release as lost requires a later unchanged confirmation or equivalent
  ordering evidence.
- **[D]** Any genuine transition clears pending missed-release suspicion before a synthetic
  repair may proceed.
- **[D]** A synthetic release repair never creates new player authority.

## 6. Runtime demand — when work is allowed to exist

- **[D]** Define cheap disabled, enabled-idle, eligible, and active boundaries per feature.
  Configuration is intent, not proof of demand.
- **[D]** Expensive scans, catalog or model work, geometry, projection, scene drawing, and
  generated-input preparation run only while the owning feature is eligible or active.
  Disabled or idle features retain no avoidable high-frequency scanning, rendering,
  enrichment, logging, or input polling.
- **[D]** Reject known-ineligible work before expensive reads. Acquire shared resources only
  after final eligibility is known.
- **[D]** Every lazy acquisition has a matching release. The last real consumer revokes
  readiness and releases polling, worker, catalog, input, and presentation ownership.
- **[D]** Shared resources use scoped leases or reference-counted demand. A monotonic
  "requested once" flag is not sufficient.
- **[D]** Eligibility stays local to the feature that owns it — no consumer inherits a gate it
  does not require.
- **[D]** Standby is event- or revision-driven. Periodic work needs measured justification and
  a bounded recovery purpose. An enabled-idle sentinel uses a measured low-frequency lane, may
  read only the minimum state needed to detect eligibility, and never invokes a heavy producer
  merely to discover there is no work.
- **[D]** Gameplay-scene producers honour a fresh presentation-demand proof bound to the exact
  foreground process identity; recovery from staleness requires newer proof for that same
  identity. They revoke camera, catalog, map, visibility, and input dependencies when the
  target is not the foreground gameplay surface, the menu owns focus, or the proof is stale —
  keeping only bounded low-rate liveness work needed for recovery.
- **[D]** A real presentation-demand transition wakes parked producers; repeated no-surface
  publication is a no-op and never becomes a polling wake.
- **[D]** A pending or stale demand generation must not authorise output. Recovery requires
  current evidence — a stale flag or single ambiguous observation restores nothing.
- **[D]** Retained immutable assets may be reused only when reuse owns no active authority and
  every identity, generation, input, and freshness the consumer needs is revalidated.
- **[D]** Per-frame, per-entity, process-read, input, and render work stays bounded and
  allocation-stable: change detection, immutable publication reuse, bounded caches, batched
  reads, explicit invalidation.
- **[D]** An equivalent visual publication refreshes freshness without allocating a
  replacement snapshot.
- Demand regression coverage **should** prove both that work starts promptly when eligibility
  appears and that it stays absent when the feature is disabled or ineligible.

## 7. Shutdown

- **[D]** Shutdown invalidates shared session authority, wakes parked task-owned workers, and
  uses a bounded registered-worker barrier.
- **[D]** Every started structured worker gets a chance to emit terminal or final partial
  evidence before the final application-log flush; that flush happens only after invalidation,
  wakeup, and the barrier have resolved.
- **[D]** Terminal diagnostic evidence is preserved. A shutdown timeout is degraded evidence,
  not success.

## 8. Cross-cutting

- **[D]** Treat process input and output actions as consequential boundaries.
- **[D]** Diagnostics may observe those boundaries but stay bounded and behaviour-neutral
  unless action changes are explicitly authorised.
- **[D]** Identity, generation, map, visibility, freshness, and authorisation gates are
  fail-closed boundaries unless an approved requirement explicitly changes one.
- **[D]** Preserve meaningful states — visible, blocked, unavailable, unknown — through
  consumers. Never collapse uncertainty into certainty for convenience.

## 9. Performance (P1)

**The game must never feel us.** No FPS loss, idle near zero, no overlay-induced stutter.
The numeric bar is per-change and lives in the gate's G6 section; the methodology is in
`docs/PERF.md`. Correctness, safety, and the invariants above come first — after them,
runtime performance is the first priority.
