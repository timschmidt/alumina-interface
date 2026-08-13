# Canonical graph component and front panel V1

`ALGC` V1 is the first reusable authoring package above `ALGW`. It embeds one
complete canonical graph workspace, gives selected internal ports a public
connector identity, and binds a bounded integer front panel to public terminals
or exact retained parameters.

## Authority boundary

Hierarchy does not weaken the existing graph boundary. The `ALGR` inside the
embedded workspace remains the only executable structural graph authority.
`ALGC` is not accepted by firmware and does not resolve an opaque node kind,
admit an implementation, flatten a component instance, allocate a resource,
prove timing, or authorize deployment. A later compiler must independently
resolve and lower a component into audited `ALGR`/graph IR before it can run.

Front-panel rectangles are signed-origin, unsigned-extent logical pixels. V1
requires nonnegative origins, nonzero extents, and bounded right/bottom edges.
They are presentation metadata only. No component API converts a rectangle into
an exact value, clock, machine coordinate, timer, or firmware quantity.

## Canonical envelope

Every fixed-width integer is little-endian. V1 contains, in order:

1. `ALGC`, version 1, and zero flags;
2. embedded component byte/count/panel-coordinate limits;
3. component revision, nonzero declared behavior version, and stable namespaced
   component name;
4. monotonic next-input, next-output, and next-panel-item identity cursors;
5. one length-delimited complete canonical `ALGW` document;
6. public inputs sorted by stable ID, each with a stable name and mapped
   internal input endpoint;
7. public outputs sorted by stable ID, each with a stable name and mapped
   internal output endpoint; and
8. front-panel items sorted by stable ID, each with a stable name, tagged exact
   binding, and integer rectangle.

Stable names are 1–64 ASCII bytes, begin with an alphabetic byte, and then use
only alphanumerics, `_`, `-`, or `.`. Public input and output names share one
connector namespace. IDs are nonzero and never duplicated. The next-ID cursors
must exceed every retained ID and may use `u32::MAX + 1` only as an exhausted
sentinel.

The first interactive policy admits at most 24 MiB total, 128 public inputs,
128 public outputs, 256 panel items, and panel edges no greater than 1,000,000
logical pixels. The separately replayed workspace retains its own 20 MiB,
256-placement, and coordinate limits. Neither embedded policy can grant itself
more authority than its caller.

`replay_graph_component` bounds the outer bytes before parsing, bounds counts
before allocation, independently replays the embedded `ALGW` and `ALGR`,
reconstructs every component invariant, rejects trailing bytes, and requires an
exact re-encoding match. SHA-256 identity is returned only after byte equality.

The initial 19-node PID/interlock component is 4,099 bytes with SHA-256
`20759fd476c435eca5318204c1048eed8156244f739bb2df5448a7f580d359d1`.
It embeds the existing 3,396-byte canonical reference workspace unchanged.

## Connector semantics

A public input maps to exactly one declared internal input. That target must
have no internal wire owner: a value has one explicit source, either the
component connector or an internal wire, never both. Multiple public inputs
cannot alias one target.

A public output maps to exactly one declared internal output. It may observe an
output that also feeds internal wires, but two public outputs cannot alias the
same endpoint. Input/output direction and exact registered value type are
resolved from the embedded graph rather than copied into a second schema. The
component exposes type queries for terminals and panel items.

Public connector inputs intentionally allow an embedded workspace to be an
incomplete editor draft. Semantic admission must account for those connector
sources when hierarchy is later flattened; `ALGC` V1 itself does not claim the
draft is executable.

## Front-panel bindings

V1 has three explicit binding kinds:

- an input control supplies one public component input at runtime;
- a parameter control transactionally replaces one exact retained node
  parameter through the normal workspace edit boundary; and
- an output indicator observes one public component output.

Each binding resolves against the embedded workspace, and a binding may appear
only once on the panel. A parameter control names both stable node and
node-local parameter IDs; its type is the retained typed value's exact root
type. An input/output item inherits its terminal's resolved graph type. Panel
layout never stores a float or display-derived value.

`GraphComponentDocument::replace_workspace` advances component revision and
validates a complete candidate before mutation. Deleting a bound node, changing
a mapped port, internally wiring a public input, or invalidating any panel
binding rejects the replacement and preserves the prior component byte for
byte.

## First editor workflow

The native/WASM control workspace constructs `control.reference_pid` version 1
around its current canonical workspace. The autonomous reference fixture has
four public Stream outputs, six exact parameter controls (P/I/D gains, clamp
minimum/maximum, and safe output), and four exact replay indicators. Controls
use the same Hyperreal parsing, typed-value validation, canonical `ALGR` edit,
history, and persistence path as the selected-node inspector.

Every accepted workspace edit rebuilds and encodes the component. If an
otherwise valid draft deletes or changes a referenced endpoint, the front panel
detaches visibly without rejecting the `ALGW` edit. Undo or another restoring
edit reattaches it after complete validation. Indicators never relabel stale
trace bytes: once the embedded graph digest differs from the reference replay,
they report that the exact replay is detached.

This initial panel metadata is reconstructed from the reviewed fixture rather
than persisted separately by the current `.algw` bridge. Canonical `ALGC` core
bytes and replay are implemented and tested; general component file exchange is
a later UI slice.

## Deliberately open

The separate canonical [`ALGH` V1 hierarchy](GRAPH-HIERARCHY-V1.md) now binds
leaf component instances by exact digest and deterministically flattens them to
ordinary `ALGW`/`ALGR`. Nested dependencies and general recursive cycle/depth
rules, editable instance workflows, component libraries, package
signatures/permissions, locked dependency manifests, connector editing,
arbitrary panel editing, panel value injection during simulation, probes,
groups/comments, and `ALGC` persistence or file exchange remain open. `ALGC` V1
grants no semantic, implementation, resource, timing, safety, firmware, or
physical-output authority.
