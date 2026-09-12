# Performance Methodology

Read before making, measuring, or claiming a performance change. These were
`.claude/rules/performance-*` until 2026-08-28. Enforcement now lives in gate **G6**; this
file is the method behind it.

The product law is one line, and it is in `docs/INVARIANTS.md` §9: **the game must never
feel us.** Correctness, safety, and product invariants come first; after them, runtime
performance is the first priority.

## What to prefer

Lower steady-state CPU, process-read traffic, render cost, allocation, synchronization, and
input latency — over feature breadth, visual polish, or implementation convenience.

A new hot-path scan, process read, render pass, allocation, thread, heartbeat, or polling
lane needs measured justification. Any proposed busy wait, unbounded poll, cache, queue,
thread, batching, or parallel-read path needs measured CPU, latency, memory,
synchronization, and failure impact before it is complete.

## Order of work

1. **Measure first.** Do not tune from source shape.
2. Optimise shared hot paths before feature-local ones.
3. Change one independently measurable thing at a time.

When current evidence cannot identify the cost owner, a scheduling or behaviour change must
be preceded by bounded, behaviour-neutral attribution. Repeated work is traced to its owner
before it is removed or reused.

## What may certify a claim

- A performance change declares, **before** changing behaviour: the claimed cost, the
  affected hot path, the baseline, the target metric or budget, and the representative
  workload. That declaration is the G6 section of the gate input.
- A claim measures the resource it claims to improve — CPU duty, wall-time tails, cadence,
  process-read calls and bytes, render time, allocations, memory growth, queue or lock wait,
  input latency, or output freshness.
- **Certification requires fresh comparable evidence from the relevant build.** Configured
  intervals, source shape, static reasoning, mocked runs, and microbenchmarks certify nothing.
- Every measurement is labelled by executable identity and evidence grade. Instrumented builds
  explain internal cost ownership; stock release builds validate shipped behaviour and
  end-to-end cost. Neither substitutes for the other, and a comparison must stay within one
  grade.
- Comparable runs use equivalent inputs, settings, duration, build profile, feature flags,
  hardware, warmup, and runtime state. Runs that differ materially cannot support a
  comparative claim and are labelled inconclusive.
- Repeat runs when the expected improvement is near normal variance; label inconsistent or
  within-noise differences inconclusive.
- Comparable evidence must distinguish every affected idle, active, worst-case, transition,
  and recovery state.
- When live evidence cannot be collected, report the change as **performance-uncertified**
  and name the exact remaining workload and measurement.
- Reuse already-authorised evidence when it provides the same proof — old session logs are a
  free baseline.

## Integrity

- Thresholds, workloads, warmup, capture duration, log scope, and enabled features must not be
  changed after seeing results merely to obtain a pass. Disclose any justified change and
  collect a new baseline.
- A result must not improve an average by hiding tail stalls, dropped work, failures,
  unavailable samples, backpressure, memory growth, instrumentation overhead, or a regression
  in another worker or state.
- An optimisation must not reduce required cadence, defer required work, reuse stale state,
  weaken proof, or collapse unknown into success in order to meet a target. Any observable
  tradeoff needs explicit approval.

## Safety of optimisations

- Outputs, safety gates, failure handling, and recovery are preserved. Proof, freshness,
  recovery, player authority, and fail-closed behaviour are never weakened for speed.
- Batching, caching, and deduplication preserve complete identity, bounded lifetime or
  capacity, explicit invalidation, and fail-closed refresh wherever stale data could affect
  correctness.
- Every optimisation that introduces a cache, schedule, budget, or concurrency boundary needs
  regression coverage proving the applicable output, safety-gate, failure, and recovery
  equivalence.

## Tools

Maintained repository performance tools should be used for the runtime paths they own —
inspect their current help and tests rather than reusing a copied invocation.

- `tools/check_scan_performance.ps1`
- `tools/build_performance.ps1`
- `tools/run_architecture_validation.ps1`
- Every session log carries an overlay diagnostic block; recent logs are the cheapest
  baseline available.

## Historical note

The 2026-08-24 architecture program's VAL-60 live A/C session was closed as superseded on
2026-08-28: months of development built irreversibly on its frozen candidate mooted the
retain/revert decision it existed to inform. Its methodology is the file you are reading,
applied per change through G6 instead of per era. No performance claim should be cited from
that program — it never produced a live-validated one.
