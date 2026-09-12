# Current Architecture

Status: static architecture implementation and final release artifacts complete; live A/B
certification pending

Last source review: 2026-08-24 against final B source manifest
`66159BF2CAA04F19FF94562C97D45702BB0C07E424D3D24F22EB1E4B59AE3F19`; executable identities are
recorded in [`VAL-60-final-candidate.md`](plans/2026-08-24-cs2-architecture/evidence/VAL-60-final-candidate.md)

This document is deliberately short. It helps a reader find the current owners, but it is not a
behavioral contract and must not replace source tracing. If this map conflicts with current source,
tests, configuration, or fresh runtime evidence, those current artifacts win and this file must be
updated.

## Runtime flow

```text
main
  -> entry_point: startup, capability construction, worker supervision, shutdown
     -> core: settings and shared immutable/runtime publications
     -> cs2: typed game reads, snapshots, resources, catalogs, visibility
     -> hacks: Aim, Trigger, ESP/world and Nade feature workers
     -> platform: generated-input authority, Raw Input and timing boundaries
     -> presentation: resources, scene/menu composition, overlay worker and Win32 backend
     -> diagnostics: typed Aim and performance evidence
```

Feature workers consume current settings and CS2 evidence, then publish bounded shared state for
the overlay or request generated output through the platform input boundary. Resource acquisition
and recovery are coordinated through `GameResourceCoordinator`; generated combat output is
serialized through `InputGate`. The application-level Auto Accept feature is retired: current
source has no feature worker or generated-click route. Legacy compatibility decoding and vendored
dumper internals are not application feature authority.

## Module ownership

| Path | Current owner |
| --- | --- |
| [`src/main.rs`](../src/main.rs) | Windows executable entry; delegates immediately to `entry_point::start`. |
| [`src/entry_point.rs`](../src/entry_point.rs) and [`src/entry_point/`](../src/entry_point/) | Startup, single-instance/elevation boundaries, feature supervision, fatal and clean shutdown. |
| [`src/core/`](../src/core/) | Settings, configuration, menu state, shared runtime/visual publications, UI-facing feature health. |
| [`src/cs2/`](../src/cs2/) | Typed CS2 state, per-tick/staged capture, offsets, resources, catalogs, map and visibility evidence. |
| [`src/hacks/`](../src/hacks/) | Runtime feature workers and their feature-local state machines. |
| [`src/echo/`](../src/echo/) | Aim selection, geometry and control logic kept separate from process/Win32 ownership. |
| [`src/platform/`](../src/platform/) | Operating-system input, Raw Input lifecycle and precise-timing boundaries. |
| [`src/presentation/`](../src/presentation/) | Presentation resources, scene/menu composition, overlay input/worker lifecycle and the layered Win32 backend. |
| [`src/diagnostics/`](../src/diagnostics/) | Structured Aim diagnostics and runtime performance attribution. |
| [`src/utils/memory.rs`](../src/utils/memory.rs) | Target-process handle and typed read primitives used by the game-reader layers. |

## Shared boundaries

- `MenuState` publishes coherent settings and feature capability/health state.
- `RuntimePublications` carries bounded publications between producers and the overlay without making the
  renderer a game-memory reader.
- `GameResourceCoordinator` owns resource demand, session generations, recovery, and retirement.
- `InputGate` is the serialized boundary for generated combat output.
- Identity, generation, freshness, visibility, map/resource proof, and unknown states remain
  fail-closed as required by `CLAUDE.md`.

## Maintenance trigger

Update this file in the same change when a top-level module gains or loses ownership, a shared
publication/input boundary moves, or startup/shutdown data flow materially changes. Do not record
poll intervals, test counts, temporary paths, or implementation proposals here; those drift too
quickly and belong in source, fresh evidence, or `docs/plans/`.
