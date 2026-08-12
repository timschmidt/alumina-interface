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
reduced rational derivations. Missing, zero-rate, nonreduced, and recursive
clocks reject. Event/stream type clocks must resolve in the complete document.
Ports are canonicalized by local ID. Wires must begin at an output, end at an
input of the identical registered type, and uniquely own the input.

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

V1 is structural, not executable. Subgraphs/components, explicit state and
feedback, queue/backpressure semantics, rate transitions, cases/loops/state
machines, front panels, resource claims, node admission, combinational-cycle
analysis, fixed-memory lowering, WCET/deadline analysis, protocol bridges,
firmware opcodes, and deterministic graph simulation remain later M9 slices.
No arbitrary graph document is sent to or interpreted by firmware.
