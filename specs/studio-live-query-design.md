# Studio live Query design

This document records the implemented Studio transport boundary for live Query answers. It does
not change GraphLoom Core Query semantics, GraphRAG compatibility, Prompt or Context construction,
provider calls, Query usage accounting, or the persisted Explainability schema.

## Runtime ownership

```text
GraphLoom Core                       graphloom-studio                    Browser
┌──────────────────────┐            ┌───────────────────────────┐       ┌────────────────────┐
│ api::query_stream()  │  Context   │ ignore for answer channel │       │ Explainability UI  │
│                      ├───────────▶│ (evidence already recorded)│       │                    │
│ QueryEventStream     │  Token     ├───────────────────────────┤ SSE   │ live answer hook   │
│                      ├───────────▶│ QueryAnswerLiveHub         ├──────▶│ snapshot + deltas  │
│                      │ Completed  ├───────────────────────────┤       │                    │
│                      ├───────────▶│ QueryResultRegistry        ├─GET──▶│ canonical result   │
└──────────────────────┘            └───────────────────────────┘       └────────────────────┘
```

Explainability remains an independent persisted semantic evidence stream:

```text
Core Explainability Sink
        │
        ├──▶ Explainability Store ── replay ──┐
        │                                     ├──▶ Explainability SSE ──▶ semantic timeline
        └──▶ Explainability Live Hub ── live ─┘
```

Answer deltas are transient and are never written to the Explainability Store. The final
`StudioQueryResult` remains the canonical answer and usage contract.

## Subscription and completion ordering

```text
Browser                QueryAnswerLiveHub             Query executor          Result / Run stores
   │                            │                            │                         │
   │ GET answer events          │                            │                         │
   ├───────────────────────────▶│ subscribe receiver         │                         │
   │                            │ capture current state      │                         │
   │◀──── snapshot(seq, text) ──┤                            │                         │
   │                            │◀──── append Core delta ────┤                         │
   │◀──────── delta(seq) ───────┤                            │                         │
   │                            │                            ├── insert pending result ▶│
   │                            │                            ├── complete Run ────────▶│
   │                            │                            ├── publish result ───────▶│
   │◀────── completed(seq) ─────┤◀──── complete answer ─────┤                         │
   │ GET canonical result       │                            │                         │
   ├────────────────────────────────────────────────────────────────────────────────▶│
```

Receiver creation precedes snapshot capture while the per-run map entry is read-locked. A writer
therefore happens wholly before the snapshot or wholly after receiver creation; events at or below
the snapshot sequence are discarded. Broadcast lag recovers from a fresh authoritative snapshot.

Every delta and terminal transition increments the per-run sequence exactly once. SSE frame IDs
equal payload sequences. Completed/failed states are retained in bounded FIFO order; active runs
are bounded independently by Studio Query admission.

On success, the result is inserted as pending before Run completion. Pending entries are fetchable
once Store metadata becomes terminal but are excluded from FIFO eviction. After Run completion the
registry publishes the result into bounded recent retention, then answer completion is emitted.
Pending entries are bounded by Query admission in addition to the published retention bound. Store
metadata terminal and answer terminal therefore both observe a ready canonical result. Core may
emit its persisted Explainability terminal envelope before Studio receives `QueryEvent::Completed`;
that earlier fallback can observe 202, so the frontend performs a short event-triggered bounded
reconciliation until either Store metadata or the answer terminal confirms readiness. A
streamed/final mismatch is never fabricated as a delta: the canonical response replaces retained
snapshot text at the terminal transition and the frontend also replaces its rendered buffer after
fetching the canonical result.

## Query mode boundary

- Basic and Local publish only their final completion provider deltas.
- Static and Dynamic Global publish only final Reduce deltas; rating and Map responses remain
  Explainability evidence.
- DRIFT publishes only Final Synthesis deltas; HyDE, Primer, and exploration action responses remain
  Explainability evidence.

These claims are guarded by the Core `api_query` streaming regressions and Studio executor tests.

## Browser auto-follow ownership

The Query workspace follows layout rather than transport fields, so Explainability growth, Answer
Markdown reflow, Analysis collapse, citations, and canonical-result reconciliation share one path.
The Radix Viewport remains the only scroll owner; the shell, page, and composer are outside this
controller.

```text
Query content resize                    Viewport scroll
        │                                     │
        ▼                                     ▼
  ResizeObserver                       distance from bottom
        │                                     │
        ▼                                     ├── near latest ──▶ follow
  should follow?                             │
        │ yes                                 └── away ─────────▶ pause
        ▼
 coalesced animation frame
        │
        ▼
 Radix Viewport scroll latest
```

Programmatic scroll events are guarded and cannot be interpreted as a reader leaving the latest
region. Content resize never changes follow intent. A real reader scroll updates intent from the
Viewport metrics, and returning within the 96-pixel threshold resumes following. Switching Runs
resets the controller and disconnects the prior observer and listeners. Machine following uses
immediate scrolling; the explicit **Back to latest** action uses smooth scrolling unless reduced
motion is requested. Manually expanding Analysis pauses following so the next Answer delta does not
pull the reader away from the requested evidence.
