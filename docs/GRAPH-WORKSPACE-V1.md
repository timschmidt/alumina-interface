# Canonical graph workspace V1

`ALGW` V1 is the first saved editing envelope around an exact `ALGR` graph. It
exists because executable structure and human canvas state have different
authority. The workspace embeds the complete canonical graph bytes, then adds
only bounded presentation metadata and monotonic editor identity cursors.

## Authority boundary

The embedded `ALGR` remains the sole input to semantic analysis, exact
simulation, deployment lowering, and trace identity. `ALGW` coordinates are
signed integer logical pixels. They can position a node on a canvas, but no API
converts them into a graph literal, exact geometry, a machine lattice, a timer,
or a firmware value.

Browser pointer motion is a lossy presentation input. A drag is rounded once at
commit into the integer canvas lattice and then admitted by the workspace
coordinate bound. The UI projects those bounded integers back to `f32` only for
egui painting. This two-way presentation conversion never crosses into the
embedded graph.

## Canonical envelope

Every fixed-width integer is little-endian. V1 contains, in order:

1. `ALGW`, version 1, and zero flags;
2. embedded workspace byte/count/coordinate limits;
3. a monotonic workspace revision;
4. monotonic next-node and next-wire identity cursors;
5. a length-delimited complete canonical `ALGR` document; and
6. node ID plus signed `i32` x/y for every graph node, sorted by node ID.

The next-ID cursors must be strictly greater than every retained ID. They may
equal `u32::MAX + 1` only as an explicit exhausted sentinel, so deleting the
largest wire cannot silently reuse its stable identity. Every graph node has
exactly one placement; missing, duplicate, zero, or foreign placements reject.

The first UI admission policy is 20 MiB total bytes, 256 placements, and an
absolute coordinate magnitude of 1,000,000 logical pixels. The embedded policy
cannot grant itself more memory than the caller's admission policy. Replay
bounds the outer bytes before reading lengths, independently replays the
embedded `ALGR`, reconstructs all invariants, re-encodes every byte, and assigns
SHA-256 identity only after exact byte equality.

The representative 21-node/25-wire PID/interlock workspace is 3,755 bytes with
SHA-256
`bf5135c39b67c46a3a5908d4d0d8a1d13d065b59231890e8fbdda818f064ae16`.
It embeds graph identity
`96a3348264a9b65d267b45f9a6419a44ee60473fd961abcf4436295e10b3735f`.

## Transactional editing

`GraphWorkspaceDocument` exposes eight transactional edits:

- move one node, advancing only workspace revision;
- create one complete node prototype and placement with the monotonic node ID;
- delete one node, its placement, and every incident wire without rewinding
  either ID cursor;
- connect one typed output to one unowned input with the monotonic wire ID;
- disconnect one existing wire without rewinding the ID cursor;
- replace one bounded canonical UTF-8 node label while preserving kind, ports,
  parameters, domain, placement, and stable identities;
- replace one concrete execution domain while preserving every other node and
  workspace fact; and
- replace one exact parameter value while preserving its stable parameter ID,
  name, and registered root type.

Each operation constructs and validates a complete candidate before replacing
the prior document. Exact placement, label, domain, and parameter no-ops do not
advance a graph or workspace revision. Rejected coordinates, missing IDs,
exhausted counters, wrong port direction/type, duplicate target ownership,
invalid label/domain, parameter type drift, or revision overflow leave the
workspace byte-for-byte unchanged. Changes to node, wire, label, domain, and
parameter facts also advance the embedded graph revision and therefore change
its canonical digest. Labels are
canonical human metadata, but are never matched as behavior identity. The core
domain replacement is deliberately structural: allowed kind/domain families,
clock relationships, wire crossings, implementations, target capabilities, and
deployment remain separate audited obligations.

The native/WASM control workspace initializes one canonical `ALGW` from the
audited deterministic layout. Its 16-entry palette is derived from the fixed
simulation registry: kind/version, ports, and parameter contracts come from
the audited node schema, while each exact initial parameter value comes from a
reviewed representative instance or an explicit false-safe Boolean default. A
kind without a fixed implementation or reviewed default prevents the palette
from opening.
The palette never manufactures an implicit resource, device domain, parameter,
or port.

Nodes can be created, selected, deleted, and dragged; an output then input can
be clicked to connect, and an input can be secondary-clicked to disconnect.
The selected-node inspector edits the bounded canonical label and offers only
execution-domain families admitted by that node's reviewed schema. Concrete
device identities are collected from existing device-cycle clocks and node
placements in the graph, sorted canonically, and never accepted as raw text.
The complete candidate is rerun through the fixed semantic registry before it
can commit. A graph with no reviewed device identity consequently cannot invent
a Service or Realtime target.

The selected-library definition editor applies that same palette, exact node
metadata, integer placement, and typed-wire surface to the embedded `ALGW` of
one non-authoritative `ALGC` dependency. It keeps separate selection, drag,
pending-wire, and text-draft state from the complete-session control canvas.
Component-instance placeholders can be selected, moved, relabeled, and wired,
but cannot be removed by ordinary node deletion because their scoped occurrence
records belong to `ALGH`. The child-component selector creates a fresh
placeholder and scoped binding atomically from an existing library dependency;
the placeholder inspector can rebind that exact occurrence through the same
selector while retaining the node, label, placement, allocation cursors, and
stable-ID-compatible live endpoints. A same-shape child leaves the parent
`ALGC` exact while changing its scoped binding; a shape change recursively
replaces affected parent identities. The inspector can instead remove the
node, its incident wires, and the binding in one transaction. Live public
connector or panel bindings reject incompatible rebinding or removal, cycles
and limits reject atomically, and deleted identities are never reused. Every
accepted definition edit freshly validates and flattens `ALGH`, regenerates
`ALGM`, and records one complete `ALGS`; control `ALGW`, probes, and cached jobs
remain exact unless a coordinated control-authority replacement is explicitly
required, which this surface rejects.

Reusable nested definitions cross project/library boundaries through canonical
[`ALCP` V1](GRAPH-COMPONENT-PACKAGE-V1.md), not by guessing bindings from a
standalone `ALGC`. The package contains no root placement and importing it does
not mutate this workspace; only a later explicit root or nested occurrence
authoring action places the imported definition.

The separate ALGM source browser is read-only navigation across those editors.
It verifies a chosen final node or wire still has the displayed exact origin,
resolves a component occurrence path through current scoped bindings, and
focuses the matching root, library-definition, or authoritative-control
canvas. Exact component wires receive a distinct local-wire highlight while
their target nodes remain available in the inspector. These selections,
highlights, and one-shot scroll requests are transient; they do not revise an
`ALGW`, replace an `ALGC`, record an `ALGS` history state, or dirty persistence.

The shared parameter surface accepts bounded schema-directed Boolean,
exact-rational, measurement-interval, canonical signed/unsigned lattice-count,
text, byte, array, record, option, and result literals. Every current
representative-control parameter is exact rational. Hyperreal parses rational
text exactly, composite parsing follows the registered schema, and canonical
`ALGR` encoding stores the normalized value without a floating-point
conversion. Resource/job handles and runtime Event/Stream shapes remain
visibly read-only in the text editor. The separate capability-derived target
draft can replace a root or nested resource handle only through exact catalog
selection and a bounded schema-aware path. Its current two-field resource
record is consumed by a reviewed Realtime conjunction node, connected to a
required sink, and re-lowered into a capability-admitted fixed firmware package
after each edit. The
separate [cache-derived job draft](GRAPH-CACHED-JOB-CATALOG-V1.md) can add or
replace an inert job handle at a bounded schema-aware composite path only after
canonical CAM artifacts and complete participant cache-ready observations match
exactly; it accepts no raw identity or path text.

The UI rebuilds semantic layering and reruns audited analysis after each
candidate. A newly created or disconnected required input is retained as an
explicit semantic blocker rather than hidden or repaired. A current-tick
combinational cycle that cannot be laid out is rejected without mutation. An
empty draft remains renderable and can accept a new palette node.

The existing `ALGT` reference trace remains visible after placement-only edits
because its embedded `ALGR` digest is unchanged. Any node, wire, label, domain,
or parameter edit detaches and hides that trace: trace bytes are never relabeled
as evidence for a graph they did not simulate. Reset reconstructs the reviewed
reference graph and layout.

## Canonical history

Undo and redo retain complete canonical [`ALGS` V1 authoring
sessions](GRAPH-AUTHORING-SESSION-V1.md), not mutable UI deltas, inverse
operations, or a subset of the visible state. One timeline therefore restores
the control `ALGW`, bound `ALGP`, catalog-bound cached-job `ALGW`, selected
`ALGC`, complete `ALGH`, and exact `ALGM` atomically. A successful graph,
probe, trigger, sidecar-import, cached-job, component-library, or direct
root-instance edit—including root placement, typed root wiring, exact
front-panel binding/layout authoring, and selected-library definition
edits—records the exact prior session and
discards the abandoned redo branch. An exact no-op records nothing.

Every navigation target first replays the complete `ALGS` and all nested
artifacts. The UI then rebuilds layout, reruns audited semantic admission, and
checks every cached-job leaf against the freshly derived catalog before
history state changes. A corrupt, noncanonical, unregistered, unbound, stale,
foreign, or otherwise inadmissible target leaves the current authoring session
and both history stacks unchanged. Navigation clears transient selection,
wire, drag, parameter-text, and probe-metadata state and resets the cursor from
the restored trigger resolution.

The default policy keeps at most 16 complete sessions in each direction and at
most 64 MiB of combined canonical `ALGS` bytes across both stacks; the
separately held current session does not count against that budget. Oldest
complete sessions are evicted without splitting nested artifacts. History is
ephemeral and is not nested inside `ALGS` or browser storage, avoiding
recursive snapshots. A freshly opened, persisted, or imported complete
session therefore begins with empty undo and redo stacks. Focused valid
`ALGW`/`ALGP` imports are ordinary undoable complete-session edits.

## Persistence and file exchange

The browser stores one canonical
[`ALGS` V1 authoring session](GRAPH-AUTHORING-SESSION-V1.md) after a successful
edit. That artifact binds the current control workspace, exact graph-probe
sidecar, catalog-bound cached-job workspace, selected component identity,
complete hierarchy, and regenerated hierarchy source map. The origin-local
value is `algs1:` followed by one lowercase hexadecimal `ALGS`; one `setItem`
replaces the whole session. The retired `algwb1:` representation and key are
not compatibility inputs.

Restore replays the complete outer artifact and every nested canonical
artifact, proves the ALGP/control binding and selected-component/control
binding, freshly flattens ALGH to regenerate ALGM, admits both visible graphs
through the fixed UI registry, audits every dependency's ordinary-node draft
through that same registry, and checks every cached-job handle against the
newly derived exact catalog. Only after all checks succeed does any in-memory
state change. Malformed, mismatched, stale, foreign, or substituted input thus
rejects the complete session atomically.

Browser download writes byte-for-byte canonical `.algs`, `.algw`, `.algp`,
`.algc`, `.alcp`, or `.algm` content. Browser upload
checks the advertised file size, bounds the materialized `ArrayBuffer`, and
forwards only bytes within that artifact's policy. The native shell exposes
independent explicit paths with bounded reads and exact, synchronized writes.
Neither platform bridge parses an artifact. A complete `.algs` import commits
only after the same atomic restore transaction used by browser persistence. An
opened `.algp` is replayed
against the exact current workspace and cannot mutate the graph; an identity
mismatch leaves both graph and prior sidecar unchanged. Explicit ALGW files
retain the 20 MiB workspace admission ceiling; ALGP files use their canonical
2 MiB document ceiling; complete ALGS files use the 8 MiB session ceiling.

A focused `.algc` import admits one standalone leaf into the current hierarchy
library only after bounded component/workspace/graph replay, exact re-encoding,
audited ordinary-node semantics, and complete hierarchy-context validation.
Successful add/remove changes are ordinary complete-session edits; an exact
duplicate import is a no-op. A dependency cannot be removed while named by a
child or parent-scope binding, and the selected control-authority component is
never removable through this workflow.

A focused `.alcp` import instead admits one selected root plus its complete
transitive definition/binding closure. It never replaces an existing component
or scoped binding. A stable-name conflict or a different child for an existing
parent/node rejects the cloned hierarchy, while an exact duplicate only follows
the transient library selection. A changing merge reruns complete recursive
flattening, source-map generation, semantic admission, and `ALGS` construction
before one historical commit.

The library can also create a canonical component without an input file. A
valid stable name deterministically produces a version-1 empty `ALGC` whose
embedded `ALGW` and `ALGR` share the current control graph's exact schema and
clocks, with every monotonic identity cursor initialized to one. It is selected
immediately and can receive its first ordinary definition node through the
same transactional authoring path. Invalid, overlong, or conflicting names
reject atomically; an already-present byte-identical empty definition is a
selection-only no-op.

The adjacent selected-component identity editor can then replace the stable
name and declared behavior version of any selected dependency. It accepts only
canonical nonzero decimal `u32` version text, permits the version to remain or
increase, and rejects regression or a stable-name collision. Accepted metadata
changes use complete recursive `ALGC`/`ALGH`/`ALGM` replacement and one `ALGS`
history transaction; an identical pair records no history or persistence
write. No compatibility alias or alternate lookup path is created.

All opened ALGW bytes first pass canonical replay, embedded-limit checks, exact
re-encoding, UI layout admission, and the fixed audited semantic registry. A
required input may remain disconnected as a visible editor draft blocker;
other semantic failures, including unknown node kinds, reject without mutation.
Thus core `ALGW` can still preserve opaque future nodes, while this particular
control UI never claims it can safely interpret or edit them.

## Current exclusions

Selection sets, groups/comments, collaborative diffs, and conflict-aware shared
persistence remain later slices. The separate cache-derived job workspace now
provides the first exact inert `JobHandle` selection path. One executable
two-input stable-Boolean composite consumer is closed; broader composite
resource shapes, prepare/start nodes, multi-job execution workflows, and live
execution authority remain open.
The separate canonical
[`ALGC` V1 component package](GRAPH-COMPONENT-V1.md) now embeds an unchanged
`ALGW` and adds a connector pane plus exact front-panel bindings; those facts
are deliberately not smuggled into this workspace format. `ALGW` grants no
semantic admission, implementation, Service/Realtime opcode, resource,
deployment, safety, or physical-output authority.
