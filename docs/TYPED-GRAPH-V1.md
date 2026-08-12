# Typed graph document V1

The first M9/I6 slice establishes a window-free graph authority in
`alumina-interface-core`. It replaces none of the firmware safety or execution
machinery and imports no old interface graph schema. The current slice is the
saved structural document on which the compiler, simulator, editor, and
capability-generated palette can be built.

## Exact value registry

Every document carries a canonical, bounded registry of units and value types.
Units have stable integer identities, seven SI base-dimension exponents, and an
exact positive rational scale relative to SI. There is no implicit unit
conversion and no `f32`/`f64` value variant.

V1 admits:

- booleans, exact rationals, and exact rational measurement intervals;
- signed and unsigned canonical integer lattices with exact positive quanta;
- bounded UTF-8 text, bytes, arrays, and records;
- options and typed success/error results;
- runtime-only events and bounded streams with explicit clock identities;
- capability-derived resource handles bound to device, board-package digest,
  resource class, and selector; and
- immutable job handles bound to device, global-job digest, and local-partition
  digest.

Composite types are stable-ID references. Construction rejects missing
references, recursive type definitions, zero/duplicate identities, malformed or
duplicate names, invalid units/quanta, excessive type depth, and every declared
or global allocation limit. Literal validation separately bounds nesting depth,
total value nodes, text/bytes/array sizes, rational numerator/denominator decimal
digits, record shape, and handle identities. Events and streams deliberately
have no saved literal.

## Structural document

The document retains a monotonic editor revision, registered clocks, opaque
versioned nodes, typed input/output ports, exact typed parameters, execution
domains, and output-to-input wires. Unknown node kind names and versions survive
load/save unchanged; resolving them into executable behavior is a later compiler
registry decision.

The three domains are `HostExact`, `Service(device)`, and `Realtime(device)`.
This field states requested placement only. It does not admit an opaque node to
firmware or prove fixed memory, WCET, resource ownership, or safety behavior.

Clocks are explicit host monotonic counters, physical-device cycle counters, or
reduced rational derivations. Each HostMonotonic or DeviceCycle clock is an
independent root with no implied offset/phase relation to another root, even at
the same frequency. A Derived clock shares its source's tick-zero epoch.
Missing, zero-rate, nonreduced, and recursive clocks reject. Event/stream type
clocks must resolve in the complete document. Ports are canonicalized by local
ID. Wires must begin at an output, end at an input of the identical registered
type, and uniquely own the input.

## Audited semantic admission

Opaque node identity remains saveable data until a separate
`GraphNodeRegistry` resolves the exact name/version. Each audited `NodeSchema`
declares the complete input, output, and parameter shape; allowed HostExact,
Service, and/or Realtime domain families; and a dependency entry for every
output. The dependency entry lists every input that can affect that output in
the current tick. Omitting an output dependency is an invalid schema rather
than an implicit assumption.

An optional `NodeStateContract` names one state type, deterministic run-start
parameter, next-state input, prior-state output, update clock, and declared
storage ceiling. The prior-state output must have no current-tick feedthrough;
updates are read-before-write.

Static storage analysis separately derives the maximum canonical byte count for
every registered literal type by checked recursion over its complete value
domain. It accounts for rational digit policy, length/tag/field overhead,
maximum text/blob/array sizes, record/option/result alternatives, and handle
identities. Event and stream reports instead bound one complete typed payload or
sample and retain clock/capacity authority; a runtime type nested inside a saved
literal composite rejects. A state declaration must hold the full canonical
typed-value bound, and reports retain declared, required, and total bytes.
This proves canonical retained-value storage, not the in-memory layout of
`hyperreal`, a future fixed firmware representation, or execution WCET.

The registry is bound to the complete canonical unit/type registry and clock
set from the document that established its authority. Analysis rejects a
different context before interpreting any document-local ID. It then rejects
unresolved kinds, shape or domain contradictions, state-policy overflow, and
all current-tick combinational cycles. Cycle analysis is iterative and bounded
over exact port vertices. Its witness identifies every structural wire and
audited input-to-output feedthrough in deterministic traversal order. A cycle
through an explicit read-before-write state output is accepted because there is
no current-tick input-to-output edge.

This is still host-side semantic analysis. A Realtime domain in a passing
report is not a firmware opcode, implementation binding, WCET proof, resource
claim, deployment package, or authority to execute.

### Input delivery and queue memory

Every audited input has a canonical delivery contract and is explicitly
required or optional. A required unconnected input rejects; an optional
unconnected input allocates nothing. Ordinary literal values use one
synchronous typed-value slot. Synchronous wires may not cross concrete
HostExact/Service/Realtime/device ownership, so an apparent scalar bridge cannot
become invisible shared state.

Event and Stream inputs instead require bounded queues. Their contract fixes
capacity and one of `Backpressure`, `Fault`, `DropNewest`, or `DropOldest` when
full. Stream queue capacity may not exceed the registered Stream type's own
capacity. Each queue item reserves the proven typed payload/sample ceiling plus
16 canonical analysis bytes: one `u64` source-clock tick and one monotonic `u64`
sequence. Per-input and total bytes are checked against independent policy and
retained in canonical node/port order. This is the host analysis/storage
contract; firmware bridge and queue layouts must later match or conservatively
exceed it before deployment.

### Exact rate transitions

Every cross-clock current-tick dependency between runtime ports is explicit.
The first admitted transition is Stream-to-Stream with an identical registered
sample type and a required bounded input queue. `LatestAtOrBeforeSourceFirst`
consumes all source samples due at or before each target tick, emits the newest,
and orders the source first when the two tick grids coincide at run start. A
missing contract, optional transition input, Event/Stream-family change, sample
type change, or gratuitous same-clock transition rejects during registry
construction.

The transition contract is audited registry authority selected by the saved
node kind/version; it is not another unchecked field in `ALGR` V1. The document
continues to retain the clocks, runtime port types, node identity, and wires
needed to reproduce admission, while the separately reviewed registry supplies
the audited semantic meaning. It still supplies no implementation opcode.

Analysis resolves every clock to an exact `hyperreal::Rational` frequency and
its independent root. Transition clocks must share that tick-zero root; equal
frequencies on independent roots do not pass. The reduced source/target
frequency ratio gives the smallest repeating schedule directly. For example,
1,000 Hz to 600 Hz is exactly 5 source ticks to 3 target ticks, never a float
approximation. The required input capacity is `ceil(5 / 3) = 2` samples between
target evaluations. Both pattern dimensions and transition count are bounded.

Latest-at-or-before also retains one complete typed sample independently of the
transport queue, which is necessary when a target runs faster than its source.
That canonical sample ceiling and its graph total have a separate admission
limit. The report retains clock/root identities, exact clock frequencies,
source/target pattern ticks, queue requirement, transition policy, and retained
sample bytes in canonical order. Runtime scheduling, transport, missed-deadline
behavior, and a fixed implementation layout remain later compiler/lowering
proofs.

## Canonical bytes and replay

`ALGR` format V1 uses fixed-width little-endian integers, length-prefixed UTF-8
or bytes, and signed reduced decimal numerator/denominator magnitudes for exact
rationals. It does not depend on serde, JSON, browser-number conversion, or a
platform ABI. Embedded limits are part of document identity.

An untrusted load must call `replay_graph_document(bytes, admission)`. The
decoder:

1. enforces the caller's total-byte limit before parsing;
2. rejects every embedded limit greater than the caller's admission policy;
3. bounds every count/string/blob/rational before allocating or parsing it;
4. reconstructs the value schema and structural document through their normal
   validators;
5. rejects trailing bytes; and
6. re-encodes the result and requires byte-for-byte equality.

Only then is the SHA-256 digest returned as the canonical graph identity.
Reordered otherwise-valid definitions, unreduced rationals, alternative boolean
or option tags, and any other second encoding therefore fail instead of gaining
a second digest.

The representative golden covers every V1 type shape, all three clock kinds,
all three execution domains, an unknown node version, a typed wire, exact
negative/positive rationals, both option/result branches, and both handle
families. Its graph digest is
`d5b886c8d655fed11d0fa54fd7a37f97cb16a2bc979ee126aa09fbf98598ceb9`.

## Deliberately open

V1 remains non-executable. Subgraphs/components, general feedback structures,
queue runtime/timeout behavior, additional resampling/window policies,
cases/loops/state machines, front panels, resource claims,
implementation/opcode admission, static firmware runtime-layout proof,
fixed-memory lowering, WCET/deadline analysis, protocol bridges, firmware
opcodes, and deterministic graph simulation remain later M9 slices. No
arbitrary graph document is sent to or interpreted by firmware.
