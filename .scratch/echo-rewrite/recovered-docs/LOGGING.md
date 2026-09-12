# Diagnostic Design

Read before adding, changing, or removing a diagnostic. These were `.claude/rules/logging-*`
until 2026-08-28; they moved here because they are reference material for one kind of work,
not something every session needs in memory.

The single rule from this set that stayed in `CLAUDE.md` is the one that repeatedly worked:
**when plausible causes cannot be told apart, add a typed, bounded, behaviour-neutral
diagnostic and capture before changing behaviour.**

## Purpose

- Every production feature has maintained diagnostic coverage owned with the feature, and a
  defined root-cause trace contract for its lifecycle.
- Coverage makes the feature's availability, activation, material action, rejection or
  failure, recovery, and shutdown paths observable.
- A new or changed feature is not complete while a relevant runtime or failure path is silent.
- For a reproduced symptom, one fresh capture must let a reviewer reconstruct:

```text
session/build/settings -> activation/input -> state transition -> evidence/proof
  -> authorization/gate -> action attempt -> action result -> recovery
```

- A capture for a reproduced symptom must locate the **first** boundary where expected and
  observed state diverge.
- Logs are evidence surfaces, not control planes. Diagnostics never enable work, authorise
  output, extend freshness, or feed a production decision.

## Neutrality

- Diagnostics must not change feature output, safety gates, evidence lifetime, scheduling or
  ordering, lock ownership, input transactions, or recovery behaviour.
- A best-effort logging failure is not proof that the observed production operation failed.
- Diagnostic cleanup may match only artifacts owned by that diagnostic producer.

## Cost

- Hot-path diagnostics are non-blocking and bounded: **gate before formatting**, and use
  transitions, aggregation, sampling, or throttling instead of per-tick output.
- A hot-path diagnostic must have negligible disabled cost, bounded enabled cost, and no
  material regression to cadence, CPU, process reads, allocation, contention, latency,
  rendering, or output freshness.
- A latency-sensitive path must not do filesystem I/O, wait for a writer, take a contended
  diagnostic lock, sleep, busy-wait, retry without a bound, or create an unbounded queue or
  payload.
- Logging preserves production performance in both disabled and enabled modes.
- Diagnostic cost is verified under a representative workload unless the diagnostic is
  self-evidently outside hot paths; report the result or the remaining live measurement.

## Structure and content

- Choose the lane by purpose and cadence: routine lifecycle and results; throttled operational
  conditions; opt-in debug chains; or a bounded structured trace.
- Machine-inspected evidence uses stable event names and structured fields with explicit units.
- Each material boundary records its owning stage, prior and next state, observed outcome, and
  typed cause.
- A proof or authorisation boundary records the identity, generation, freshness, comparison, or
  unavailability that explains its decision.
- Record evidence at the owner of the observed state. Do not infer another owner's result when
  that owner can publish the typed outcome.
- Participating owners carry bounded, session-local correlation fields so one attempt can be
  followed without raw process identity. The same field name keeps the same meaning across
  correlated events.
- Never log secrets or unnecessary raw identity. Prefer bounded tokens, categories, hashes, and
  aggregates over addresses, handles, personal identifiers, private paths, or precise sensitive
  data.

## Typed states

- A supported capture distinguishes: no attempt, early rejection, pre-action suppression,
  attempted-action failure, confirmed completion, and later recovery. Meaning must not depend
  on prose or event ordering alone.
- Evidence preserves missing, dropped, stale, unavailable, unknown, false, and zero as
  **distinct** states.
- Diagnostics must not report an attempted action as completed without evidence from the action
  boundary.
- First occurrence, cause or state change, material action result, and recovery stay distinct
  events.
- Repeated steady-state conditions may be summarised only in fixed windows that retain typed
  per-cause counts, window duration, and every dropped or incomplete sample.
- Queues, buffers, files, and retention limits expose drops, overflow, truncation, writer
  failure, and incomplete shutdown evidence.

## Files and schema

- Diagnostic files have bounded ownership, names, size, count, and retention.
- **Schema versioning** (gate-enforced): a change that alters an event name, field semantics or
  units, required fields, or aggregation windows must be versioned *and* must update every
  in-repository consumer. A purely additive field still bumps the version, but keeping existing
  names and meanings identical preserves comparability with older captures — do that when a
  baseline still matters.
- Stale, legacy-schema, truncated, overflowed, or workload-incomplete logs are not complete
  proof when the missing evidence could affect the conclusion.
- Remove task-only probes, raw dumps, and scratch logs unless the user explicitly accepts them
  as a maintained diagnostic surface.

## Testing

Test the observable contract in proportion to risk: gates, throttling, serialization,
correlation, representative state sequences, drop accounting, retention, flush behaviour, and
forbidden sensitive fields.
