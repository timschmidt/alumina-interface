# Canonical hierarchy front-panel run V1

`ALFR` is the browser/WASM host-execution authority for component
`InputControl` bindings. It does not modify `ALGC`, `ALGH`, or `ALGW`, and it
does not grant deployment or firmware authority.

## Authority resolution

Each control key is a stable root-to-nested component occurrence path followed
by one `GraphFrontPanelItemId`. Resolution requires the exact `ALGH` digest and
an independently reproduced hierarchy flattening:

1. The panel item must bind a public component input.
2. That public input is followed recursively through nested component
   placeholders by stable public-input identity.
3. Flattening provenance resolves the final ordinary input endpoint.
4. The final port must be a registered `Stream`; the control value type is its
   exact sample type and the schedule clock is the Stream clock.
5. An authored structural wire makes the control inactive because the wire
   already owns the input.
6. If parent and child panel controls reach the same unowned input, the
   shallowest occurrence wins. The child remains visible as explicitly
   superseded.

An endpoint supplied directly by a caller is never sufficient authority. The
low-level simulator independently rejects connected, nonexistent, non-Stream,
mistyped, out-of-horizon, and nonmonotonic injected samples.

Public outputs have a separate read-only projection boundary. Each output key
is a stable root-to-nested component occurrence path followed by one
`GraphComponentOutputId`. Resolution requires the same exact `ALGH` digest and
independently reproduced flattening, follows public-output placeholders through
nested components, and ends at one ordinary Stream output. The public and final
value types must match exactly; the registered Stream supplies the sample type
and clock. Missing occurrences or outputs, path-limit violations, non-Stream
outputs, type drift, and inconsistent flattening fail closed.

## Canonical envelope

All integers are fixed-width little-endian and all collections are
caller-bounded. The V1 byte sequence is:

1. magic `ALFR`, version `1`, and zero flags;
2. seven embedded limits: document bytes, control count, occurrence depth,
   changes per control, total changes, expanded samples, and root ticks;
3. exact source `ALGH`, flattened `ALGW`, and simulation-registry SHA-256
   identities;
4. the independent root clock and inclusive root tick;
5. schedules sorted by occurrence path and panel item ID;
6. for each schedule, its path, panel item ID, and exact local-clock changes;
7. for each change, the local Stream clock tick and the same schema-relative
   canonical typed-value encoding used by `ALGR` and `ALGT`.

Each active control must occur exactly once, each schedule must start at local
tick zero, change ticks are strictly increasing after canonicalization, and no
change may fall after the inclusive horizon. Empty, missing, duplicated,
connected, superseded, foreign, or oversized controls fail before execution.

Replay decodes under caller ceilings, re-resolves the current hierarchy,
revalidates every exact value, reconstructs the run, and requires byte-for-byte
canonical re-encoding. There is no compatibility decoder for another version.

## Execution

V1 changes are exact sample-and-hold values. The host expands each schedule at
every tick of its own Stream clock through the inclusive root horizon. Clock
conversion uses the analyzed exact rational rates; no floating-point time is
introduced. Expanded samples retain the resolved flattened input, local tick,
deterministic sequence, and exact typed value.

The fixed host simulator consumes those samples together with ordinary
external-source samples. Its `ALGT` V2 output marks injected inputs separately
from external source outputs and modeled node outputs. Independent `ALGT`
replay therefore proves both the canonical `ALFR` expansion and the modeled
result.

Output projection additionally binds the selected `ALGT` execution to the
current `ALFR` hierarchy, flattened workspace, registry, and inclusive
horizon. Every candidate entry must match the resolved final endpoint, clock,
and exact sample type. Its local clock tick is converted to an exact rational
root-clock tick using analyzed rates. Selection returns the latest entry at or
before the shared root cursor, breaking equal-time ties by deterministic trace
sequence. A negative or post-horizon cursor, foreign execution context, or
malformed output sample rejects instead of falling back to unrelated trace
data.

The UI keeps drafts transient and exposes one bounded timeline per active
control. Tick zero is fixed and mandatory; later ticks are canonical unsigned
decimal `u64` values and must be strictly increasing. Rows use the shared exact
graph-literal parser and have add, remove, reset-to-constant, and run actions.
Any text or row edit immediately discards prior runtime evidence. A successful
run builds `ALFR`, executes the flattened graph, and reports the `ALFR` and
`ALGT` identities. Drafts and runs never enter `ALGS`, persistence, or
undo/redo.

When a selected panel has an `OutputIndicator`, the UI exposes one shared exact
root-time cursor. Accepted text is a canonical integer or reduced
`numerator/denominator` with no whitespace or alternate spelling, bounded by
the inclusive simulation horizon. Start, end, apply, and restore-accepted
actions update only transient inspection state. A current `ALFR` run projects
only its bound `ALGT`; without one, the reference `ALGT` remains explicitly
identified as the source. Cursor edits neither invalidate nor modify `ALFR`,
`ALGT`, `ALGS`, persistence, or undo/redo. CAM, motion planning, machine
scheduling, resource claims, and firmware command generation remain separate
explicit boundaries.

## Current bounds and deferred work

The interactive policy admits at most 4,096 controls, path depth 64, 4,096
changes per control, 65,536 total changes, 65,536 expanded samples, one million
inclusive root ticks, and 16 MiB of canonical bytes.

V1 supports exact schedule data, bounded multi-change timeline authoring, and
hierarchy-safe exact output-indicator projection at a shared rational cursor.
Responsive/grouped panels, permissions/signatures, and live device-side control
remain later work. Firmware never interprets `ALFR`.
