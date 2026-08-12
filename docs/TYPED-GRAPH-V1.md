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

Canvas placement is intentionally not an `ALGR` value. The separate canonical
[`ALGW` V1 workspace](GRAPH-WORKSPACE-V1.md) embeds the complete graph and adds
one bounded signed-integer logical-pixel position per node plus monotonic
node/wire allocation cursors. Placement edits cannot change the embedded graph
digest; typed wire edits reconstruct and revalidate the whole graph, advance
both revisions, and never reuse deleted identities. This separation prevents
pointer or renderer values from becoming exact-control or machine authority.

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
updates are read-before-write. A state port may carry the literal directly or
carry it as a Stream sample on the exact state clock. Only one literal is
stored; the Stream envelope and history never become hidden state.

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
and orders the source first whenever the two tick grids coincide, including run
start. A missing contract, optional transition input, Event/Stream-family
change, sample type change, or gratuitous same-clock transition rejects during
registry construction.

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

## Fixed deterministic host simulation

`GraphSimulationRegistry` is a second, explicit authority above the audited
semantic registry. It binds exact node kind/version identities to one of nine
reviewed behaviors:

- caller-supplied external Stream source;
- the audited `LatestAtOrBeforeSourceFirst` Stream transition;
- Stream sink with no modeled side effect;
- same-clock exact-rational add and subtract;
- exact scale by a dimensionless unit-bearing parameter;
- exact inclusive clamp;
- an explicit read-before-write unit delay; and
- an exact-value permit gate whose false branch always selects its declared
  safe parameter.

The registry canonicalizes bindings and hashes the complete exact unit/type and
clock context, analysis limits, audited node schemas, and fixed implementation
selections as an `ALSI` V2 identity. Simulation accepts only `HostExact` nodes,
requires an implementation for every node, and evaluates no firmware resource
or device placement. Binding the context means that changing a unit scale or a
clock definition changes the registry identity even when every node binding is
otherwise unchanged.

The caller supplies an inclusive horizon in one independent root clock and
exact typed samples with source-clock ticks and monotonic sequence numbers.
External input order is not authority: samples are canonicalized by source,
tick, and sequence, while duplicate or regressing per-source ticks/sequences
reject. All scheduling comparisons use `hyperreal::Rational` root time. Each
target tick consumes all source samples at or before it, so a coincident source
sample is visible at that target tick; a missing tick-zero value rejects rather
than creating an implicit initial value. Declared queue capacity is enforced
against each due interval and the unconsumed horizon tail.

External sample count, generated ticks per transition, total trace entries,
root-clock horizon, and canonical trace bytes have independent limits. The
same generated-tick limit also bounds each clocked control domain. Every
same-clock control input must have one exact sample at every evaluated tick;
sparse values require an explicit rate transition rather than an implicit
hold. Arithmetic output is reconstructed through the document schema at every
tick, so rational magnitude policy remains authoritative after computation.
Dimensionless scale values include their registered exact unit scale.

The unit delay is the only stateful fixed behavior. Its implementation must
match the audited `NodeStateContract` exactly: initial parameter, state type,
clock, next input, prior-state output, and bounded canonical storage. At each
tick all delay outputs expose prior state first, combinational nodes then settle
in audited dependency order, and only afterward do delays capture next state.
This admits deliberate feedback while preserving the existing combinational-
cycle rejection. No controller state is hidden in a PID-specific opcode.

The representative control fixture resamples 50 Hz setpoint, measurement, and
permit Streams onto a 10 Hz control clock. It composes subtract, two explicit
delays, exact scale/add, clamp, and permit nodes into a discrete PID/interlock.
The coefficients are percentage values with exact `1/100` unit scale. Its
integral and derivative factors are explicitly pre-discretized for that clock;
the simulator supplies no hidden or floating-point `dt`. Its
integral prior-state trace is `0, 3, 5, 6, 6, 6` mm, the clamped controller trace
is `5, 5, 4, 2, 3, 3` mm, and dropping permit forces the final trace to
`5, 5, 4, 0, 0, 0` mm. Reversing every caller sample reproduces the same
simulation, and `ALGT` replay regenerates the complete trace byte for byte.
The canonical fixture graph identity is
`fb173fb30bc5e04269caea439dea8fa455050142fac3a4afc78f5fd16e7ac59a`;
its fixed semantic/implementation registry identity is
`6bb6f814941b632ac5c9858fbbfe599fe8febb3a04b4dcc7bf4fbc8ac2f61537`.
The 7,836-byte trace has SHA-256
`4d9b63633be3afc658cac8d6475d6ede602568ab084de005ac5dd2dfcb7542a3`.

The fixture is one fallible public core construction shared by its regression
test and the native/WASM application; the UI does not reproduce its values or
topology. The initial workspace independently caps presentation at 256 nodes,
1,024 wires, and 4,096 points per selected series. It ranks nodes from audited
current-tick dependencies, excludes only declared next-state captures from that
acyclic rank, and routes those captures visibly as feedback. Its canonical
`ALGW` envelope retains one bounded integer position per node and monotonic
identity cursors. Node drags and typed wire connect/disconnect operations replace
the draft only after complete candidate validation; structural edits detach the
reference trace because its `ALGT` identity still binds the reviewed graph.
The initial 3,396-byte workspace has SHA-256
`d7d4ef9e27359a474b59f48cdbcb604b3d4d16f2a768a65f12c95dde8aee9799`.
Node selection exposes kind/version, typed ports, exact parameters, and explicit
state facts. Four traces show error, integral prior state, clamped controller,
and permit-gated output. Egui coordinates and plot labels are named display
projections from certified finite `f64` enclosures; the hover cursor displays
the retained exact rational. Headless core-edit and full-frame tests exercise
the same native/browser paths.

The fixed host subset still does not model resource handles, physical side
effects, Service/Realtime execution, deadlines, or firmware layout.

`ALGT` V1 is a canonical deterministic trace. Its fixed-width header binds the
canonical graph digest, the semantic/implementation registry digest, and the
inclusive root horizon. Every entry retains origin, endpoint, clock, tick,
sequence, and the canonical typed value. Untrusted replay bounds and decodes the
trace, extracts only external inputs, independently reruns simulation,
re-encodes the entire result, and requires byte-for-byte equality. The
representative 1,000 Hz to 600 Hz trace is 658 bytes with SHA-256 identity
`99677284550e7465541096c675ddd360416a3f3655653af3c96e6c6d96ffa2f4`.

## First fixed Service/Realtime lowering

`GraphDeploymentRegistry` is a third explicit authority. It binds reviewed
kind/versions to one fixed firmware opcode, schedule clock, and nonzero WCET.
Its `ALDI` V2 identity covers the canonical graph digest, host lowering limits,
complete audited semantic registry, and canonical implementation bindings. A
changed port, domain family, channel/full policy, dependency, rate transition,
state contract, opcode, clock, WCET, or host limit therefore changes identity.

The deployed V2 subset is intentionally smaller than the future graph system:

- a Service-domain Boolean Stream constant with one exact Boolean parameter;
- a Realtime Boolean `LatestAtOrBeforeSourceFirst` transition; and
- a Realtime Boolean Stream sink with no side effect; and
- a Realtime stable Boolean input whose parameter is one typed resource handle.

Every structural node must have one reviewed implementation, and every node
must target the same nonzero `DeviceId`. HostExact nodes, foreign devices,
explicit graph state, Event/synchronous channels, Realtime-to-Service edges,
and lossy realtime queues reject. The one admitted resource operation requires
the handle's exact device ID, board-package digest, class, and selector to match
an authenticated target capability entry with `StableBooleanInput` access. The
structural wires must topologically order; firmware never receives `ALGR` bytes.

`GraphDeploymentLimits::from_capability_document` is the production authority
for package size, record/queue bounds, all five split arenas, and the exact
opcode and graph-resource palettes. Lowering also requires the target's
capability digest to equal the independently parsed document identity. There is
no production default that guesses a board's graph limits. The `ALDI` V2
implementation identity binds the complete capability document identity and
all of those exact limits and palettes before the audited semantics and opcode
bindings.

Each active domain uses one exact graph clock. Its independent root must be the
target MCU's `DeviceCycle` clock, and the clock period must be an exact integer
number of device cycles. The compiler sums reviewed node WCET and reserves a
separate nonzero executor/queue budget; both must fit the period. This is a
static compiler bound, not measured target WCET evidence.

Audited channel reports lower to fixed arenas. The Boolean Stream item is
exactly 21 bytes: a four-byte little-endian deployment-local Boolean tag `1`,
one canonical `0`/`1` byte, one `u64` source-schedule tick, and one `u64`
sequence. The deployment tag is not the document-local `GraphTypeId`; `ALDI`
binds that schema while fixed firmware opcodes use one independently decodable
runtime type. The representative source→transition queue has capacity two and
reserves 42 Service-to-Realtime bytes; the transition→sink queue reserves 21
Realtime bytes. `BooleanLatest` separately reserves its five-byte retained
sample, for 68 selected payload bytes. Fixed package, queue-cursor, adjacency,
mutex, fault-mailbox, and executor metadata are additional compile-time storage
and are reported by the concrete firmware runtime rather than hidden in 68.

`lower_graph_deployment` builds the sibling `alumina-graph-ir` 4,096-byte
`ALGRIR02` package and immediately passes it through that `no_std`,
allocation-free independent decoder. The package binds graph, implementation,
device, capability, and configuration digests and retains topological nodes,
integer schedules/WCET/reserve, contiguous state/channel offsets, bridge
ownership, capacity, full policy, and aggregate totals. The representative
lowered package has SHA-256 identity
`9b01fc822a4aae397fc87646e31a615fe19599f49978354810fabfb97258c696`.

One native cross-repository test passes those exact bytes and identities into
the sibling `FixedGraphRuntime<0, 5, 0, 21, 42>`. It transactionally admits the
package, primes Service release tick zero before core-1 ownership, splits unique
Service/Realtime state and queues, then executes the 1 kHz constant and 500 Hz
latest/sink releases with exact expected values and no fault. This proves the
portable compiler/runtime contract. The same fixed package can now be published
to SD, independently admitted by both live firmware cores, and installed into
permanent core-local actors. `GraphRunMachine` sends one authenticated exact
future epoch, treats request acceptance separately from both actors reporting
Running, reconciles exact stop, rejects foreign run identity, and retains the
first fault report across firmware-latch reset. The pinned Embassy tasks enforce
the declared release reserve as a lateness boundary. A second cross-repository
fixture builds the complete TinyBee 8 MiB capability document, lowers a typed
GPIO33 handle to `StableBooleanInput`, and executes the emitted package through
the firmware's permanent actor types with a supplied debounced value. GPIO34,
which is a general board resource but not in the graph palette, and a foreign
capability digest reject before deployment. Firmware runtime admission rechecks
the exact opcode, resource class, access, and selector. The physical read path
still has no connected-board HIL or measured deadline/WCET evidence, and no
graph opcode can drive a GPIO or motor.

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

V1 now has one fixed host-executable Stream/rate/exact-control subset, while
deployed graph IR V2 has one capability-bound Service/Realtime lowering and
portable executor; arbitrary documents remain non-executable.
Subgraphs/components, multi-value state records, queue timeouts and additional
policies, cases/loops/state machines, front panels, capability-generated editor
nodes, broader resource claims, general host implementation admission, measured
WCET/deadline analysis, physical HIL, output and motion opcodes, and
protocol-resource nodes remain later M9 slices. No arbitrary graph document is
sent to or interpreted by firmware.
