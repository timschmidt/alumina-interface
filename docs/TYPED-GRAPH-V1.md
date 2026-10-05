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
semantic registry. It binds exact node kind/version identities to one of
thirteen reviewed behaviors across eighteen fixed kind/version bindings:

- caller-supplied external Stream source;
- the audited `LatestAtOrBeforeSourceFirst` Stream transition;
- Stream sink with no modeled side effect;
- same-clock exact-rational add and subtract;
- exact scale by a dimensionless unit-bearing parameter;
- exact inclusive clamp;
- an exact inclusive-range predicate with same-unit lower and upper parameters;
- same-clock Boolean conjunction;
- a schema-generic clocked parameter constant, currently bound for Boolean and
  exact-rational Streams;
- a schema-generic same-clock typed case with a Boolean selector and explicit
  false and true inputs, currently bound for Boolean and exact-rational Streams;
- an explicit read-before-write typed unit delay; and
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

The typed constant, typed case, and typed delay form the first explicit
state-machine subset. Every case selector is a required Boolean Stream; its
false branch, true branch, and output must be identical typed Streams on the
selector clock. Both branches must therefore be present at every tick even
though only the selected value is emitted. A reset-dominant Boolean set/reset
machine is composed visibly as `set_case = case(set, current, set)`,
`next = case(reset, set_case, false)`, and
`current = delay(next, initial=false)`. Simultaneous set and reset selects the
constant false branch. Its regression trace for `current` is
`[false, false, true, true, false, false, true]`; reversed caller sample order
and independent `ALGT` replay reproduce it exactly. These nodes remain
`HostExact` only and grant neither a deployment binding nor an output opcode.

The exact-rational specialization composes two cases and one delay as an exact
reset-dominant register. It begins at `7/3`, loads `11/5`, resolves a concurrent
load/reset to the explicit `-5/7` reset constant, and later loads `17/11`. Its
prior-state trace is `[7/3, 7/3, 11/5, 11/5, -5/7, -5/7, 17/11]`. No float,
display projection, hidden state, or type-specific evaluator participates;
reversed caller samples and independent canonical replay reproduce the same
typed values.

The representative control fixture resamples 50 Hz setpoint, measurement, and
permit Streams onto a 10 Hz control clock. It composes subtract, two explicit
delays, exact scale/add, clamp, an inclusive measurement-range predicate,
Boolean conjunction, and a fail-safe permit node into a discrete
PID/interlock. Reversed range parameters reject instead of silently swapping
bounds, and the conjunction is independently tested with disagreeing inputs.
The coefficients are percentage values with exact `1/100` unit scale. Its
integral and derivative factors are explicitly pre-discretized for that clock;
the simulator supplies no hidden or floating-point `dt`. Its
integral prior-state trace is `0, 3, 5, 6, 6, 6` mm, the clamped controller trace
is `5, 5, 4, 2, 3, 3` mm, and the exact interlock forces the final trace to
`5, 5, 4, 0, 0, 0` mm. The independently resampled external permit is
`[true, true, true, true, false, false]`; the measurement-range and combined
permit results are `[true, true, true, false, false, false]`, visibly
attributing the tick-3 stop to the range predicate. Reversing every caller
sample reproduces the same simulation, and `ALGT` replay regenerates the
complete trace byte for byte.
The canonical fixture graph identity is
`96a3348264a9b65d267b45f9a6419a44ee60473fd961abcf4436295e10b3735f`;
its fixed semantic/implementation registry identity is
`3ccd0aa5dbc2f745b897ca06e9a75717365cd977ab303e3c1ca48bdcf8f7c525`.
The 8,292-byte trace has SHA-256
`a89c8bfa87e7dced328d6cb6583a10a618431775bd3a51d2ab61a56997d4c177`.

The fixture is one fallible public core construction shared by its regression
test and the native/WASM application; the UI does not reproduce its values or
topology. The initial workspace independently caps presentation at 256 nodes,
1,024 wires, and 4,096 points per selected series. It ranks nodes from audited
current-tick dependencies, excludes only declared next-state captures from that
acyclic rank, and routes those captures visibly as feedback. Its canonical
`ALGW` envelope retains one bounded integer position per node and monotonic
identity cursors. An 18-entry palette derives kind/version and port/parameter
shape from the fixed audited schemas and exact defaults from reviewed fixture
instances, explicit false-safe Boolean defaults, or exact zero. Monotonic node creation,
atomic node/incident-wire deletion, node
drags, typed wire connect/disconnect, bounded canonical label replacement,
concrete execution-domain replacement, and exact scalar/composite literal
replacement mutate the draft only after complete candidate validation. Labels
are canonical human metadata rather than node-kind identity. The domain mutator
is structural; the inspector exposes only families admitted by the reviewed
schema and device identities already established by a graph clock or node
placement, then reruns complete audited analysis before commit. It never
accepts a raw device ID. Any embedded graph edit detaches the reference trace
because its `ALGT` identity still binds the reviewed graph; placement-only edits
preserve it.
The initial 3,755-byte workspace has SHA-256
`bf5135c39b67c46a3a5908d4d0d8a1d13d065b59231890e8fbdda818f064ae16`.
Node selection exposes kind/version, an editable bounded label, audited concrete
execution choices, typed ports, exact parameters, and explicit state facts. A
public window-free formatter/parser now supplies deterministic
schema-directed editor text: exact rationals retain Hyperreal notation, text is
quoted and escaped, bytes use `hex"..."`, arrays use `[a,b]`, records use
canonical `{field:value}` order, and options/results use explicit
`some`/`none` and `ok`/`error` branches. Boolean, measurement-interval, and
canonical integer shapes share the same boundary without floating conversion.
Whitespace and uppercase input hex normalize on commit; malformed, oversized,
misordered, schema-contradicting, and trailing input cannot mutate the draft.
One interactive field admits at most 2,097,158 UTF-8 bytes, enough for the
complete one-MiB byte-literal ceiling. Resource/job handles, including ones
nested in composites, remain read-only until an authenticated capability/cache
selector supplies identity. The separate TinyBee target draft now implements
that rule for root and nested resource handles. Stable record-field IDs,
existing option/result branches, and retained bounded-array indices select an
existing leaf; its current and replacement handles must belong to the same
exact capability-derived catalog, and the replacement must remain unique
across every root or composite parameter in the workspace. Its current
two-field record is also an executable Realtime consumer: each accepted edit
must lower to an independently decoded capability-admitted pair opcode and
required sink. Raw handle text stays closed.
The separate cached-job draft now extends that rule through bounded
schema-aware paths: stable record-field IDs, existing option/result branches,
and retained array indices can select a nested job leaf, while the current and
replacement handles must both belong to the same exact cache-derived catalog.
Broader executable composite resource behavior remains open; Event/Stream types
still have no literal. Seven
mixed-signal traces show error, integral prior state,
clamped controller, permit-gated output, external permit,
measurement-within-range, and combined permit. The first four render as
certified analog enclosures and the final three as aligned Boolean
logic-analyzer lanes on the same exact time grid. Egui
coordinates and analog plot labels are named display projections from
certified finite `f64` enclosures; the shared cursor displays the retained exact
physical scalar or Boolean value. Measurement intervals and signed/unsigned
canonical lattice counts use the same analog boundary: intervals retain exact
closed endpoints, while counts are multiplied by the registered exact quantum
and keep the original count in the cursor label. Literal non-scalars use
categorical state/event lanes instead of a numeric projection. Their exact
schema-relative typed-value bytes are retained, repeated events are
distinguished from byte-exact changes, and the cursor exposes a bounded preview
plus full SHA-256 identity, canonical byte count, and original sequence. Events
at one exact root time are visibly counted without fabricating sub-tick time.
Text, byte, array, record,
option, result, resource-handle, and job-handle roots are supported under a
64-lane and 16-MiB identity-byte display policy. Headless core-edit and
full-frame tests exercise the same native/browser paths. Each probe row now
exposes bounded canonical name, retained-sample, and event-stride fields.
Applying them transactionally retains
probe/source/type identity, rejects duplicate or malformed names and capture
policies that cannot contain the active trigger window, and treats exact
metadata reapplication as a no-op. Plot input now comes from a shared core
projection that applies those policies to exact simulation entries, bounds all
series to 131,072 aggregate samples, and preserves exact rational root-clock
time alongside original clock/tick/sequence/value. The plot now uses that exact
shared-root time directly, so same-root local clocks can coexist without
misaligning their tick integers. Pointer display coordinates snap to an
original exact sample/window time, and each series has explicit last-value-at-
or-before cursor semantics. The plot accepts every nonempty projected Boolean
or supported physical-scalar probe, including caller-owned external-source
trace records; canonical names replace the former fixed-series whitelist.
Physical scalar series share an analog scale only when their complete
registered sample type is identical; other types receive deterministic
separate panes on the shared exact root-time axis.

Bounded undo/redo stores complete canonical `ALGS` authoring sessions and
replays every nested artifact, including the sidecar's exact
external-workspace binding and cached-job catalog admission, before mutating
navigation state. Graph, probe metadata, trigger, ALGP-import, and cached-job
changes share this history; exact no-ops do not consume a snapshot. Browser origin-local storage
preserves the current control ALGW, bound ALGP, catalog-bound composite
cached-job ALGW, and optional exact component hierarchy/source map in one
canonical [`ALGS` V1](GRAPH-AUTHORING-SESSION-V1.md), while history remains
ephemeral. Browser `.algs`/`.algw`/`.algp` upload and download and the
native explicit-path bridges exchange exact bytes under their independent
8 MiB session, 20 MiB workspace, and 2 MiB probe ceilings. ALGW imports additionally require
layout admission and the fixed audited registry, allowing only a visible
missing-required-input draft blocker rather than silently interpreting unknown
behavior.

The fixed host subset still does not model resource handles, physical side
effects, Service/Realtime execution, deadlines, or firmware layout.

`ALGT` V2 is a canonical deterministic trace. Its fixed-width header binds the
canonical graph digest, the semantic/implementation registry digest, and the
inclusive root horizon. Every entry retains one of three distinct origins—an
external source output, a hierarchy-authorized injected input, or a modeled
node output—plus endpoint, clock, tick, sequence, and the canonical typed
value. Untrusted replay bounds and decodes the trace, extracts only the two
caller-owned origins, independently reruns simulation,
re-encodes the entire result, and requires byte-for-byte equality. The public
`encode_typed_graph_value` boundary exposes those same schema-relative
type-ID-plus-value bytes and their SHA-256 identity for exact in-context state
comparison. It is deliberately not a self-describing value document: portable
use must retain the containing graph/trace identity that binds the schema. The
representative 1,000 Hz to 600 Hz trace is 658 bytes with SHA-256 identity
`3048b8931db66a6bc433ab27bc8a31fd1daf308ac74192b2dccf7d54003bf195`.

Canonical [`ALFR` V1](GRAPH-FRONT-PANEL-RUN-V1.md) is the stronger authority
above injected inputs. It resolves `InputControl` bindings through exact
hierarchy occurrence paths and flattening provenance, expands exact
sample-and-hold changes on each Stream clock, and emits an ordinary replayable
`ALGT` V2 result without granting firmware authority. Public component outputs
resolve recursively through the same exact hierarchy to their final Stream
endpoint. The front panel then projects the latest bound trace sample at or
before one transient exact rational root-clock cursor, with exact rate-domain
conversion and no mutation of `ALFR`, `ALGT`, `ALGS`, persistence, or history.

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
- a Realtime Boolean Stream sink with no side effect;
- a Realtime stable Boolean input whose parameter is one typed resource handle;
  and
- a Realtime stable Boolean pair conjunction whose parameter is one exact
  two-field record of distinct, same-class resource handles.

Every structural node must have one reviewed implementation, and every node
must target the same nonzero `DeviceId`. HostExact nodes, foreign devices,
explicit graph state, Event/synchronous channels, Realtime-to-Service edges,
and lossy realtime queues reject. The admitted resource operations require each
handle's exact device ID, board-package digest, class, and selector to match an
authenticated target capability entry with `StableBooleanInput` access. The
pair immediate carries both canonical 32-bit selectors in stable field order;
runtime admission and execution validate/read both even when the first value is
false, and absence of either faults before emission. The
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
the firmware's permanent actor types with a supplied debounced value. A second
fixture lowers ordered GPIO22/GPIO35 handles to `StableBooleanPairAll`, observes
both runtime reads on every release, and proves false then true conjunction
results. Duplicate pair members, GPIO34,
which is a general board resource but not in the graph palette, and a foreign
capability digest reject before deployment. Firmware runtime admission rechecks
the exact opcode, resource class, access, and selector. The physical read path
still has no connected-board HIL or measured deadline/WCET evidence, and no
graph opcode can drive a GPIO or motor.

## Capability-derived editor nodes and bounded probes

The editor no longer manufactures physical handles from a nominal pin name.
`derive_graph_capability_node_catalog` intersects the complete authenticated
graph-executor capability section with the separately reviewed deployment
registry. Only a resource whose opcode, Realtime domain, class, access, and
support all match can produce a concrete `GraphNodePrototype`. Its exact device
and capability identities, resource class, and canonical typed selector are
already present in the read-only parameter. Structural insertion and every
later semantic/deployment proof still run normally.

The visible TinyBee 8 MiB reference catalog therefore exposes only
GPIO22/32/33/35 with `StableBooleanInput` access. Its separate Realtime
workspace starts with an executable GPIO22/GPIO32 pair feeding a required sink,
uses explicit offline reference device/configuration identities, and shows both
canonical ALGW and lowered fixed-package identities. ADC, UART, timer,
shifted-output, storage, other GPIO, and raw pin access remain closed even though
the broader board descriptor knows about them.
See [`GRAPH-CAPABILITY-CATALOG-V1.md`](GRAPH-CAPABILITY-CATALOG-V1.md).

Canonical `ALGP` V2 is a bounded presentation sidecar. It binds stable probe
IDs/names and capture-retention ceilings to exact output endpoints and one
canonical `ALGW` digest. One optional Boolean-stream rising, falling, or either
edge trigger names a stable probe and bounded pre/post retained-sample counts.
Replay resolves output direction and exact value type, enforces caller and
embedded limits, and requires byte-for-byte canonical re-encoding. Exact host
resolution separately verifies the simulation graph identity and reports the
first matching clock/tick/sequence and available window using bounded waiting
state. Probe/trigger edits are transactional, never reuse IDs, and never mutate
the graph. The 407-byte seven-series reference sidecar has SHA-256
`50955da7b4464a02f6e3eace1d51a3e9d08f56cdfa9c661259c33195b15ef223`.
Its probe-5 falling edge matches control tick 3 and selects exact ticks 1–5. It
filters immutable host trace series only; it grants no firmware read, telemetry,
device-trigger, or deployment authority. See
[`GRAPH-PROBE-V2.md`](GRAPH-PROBE-V2.md).

The browser stores one exact `ALGS` authoring session and restores nothing until
the outer artifact, both workspaces, probe binding, optional hierarchy/source
map, selected component, UI semantics, and cached-job catalog membership all
succeed. Bounded `.algs`, `.algw`, and `.algp` exchange is available in native
and browser shells; an ALGP import can change only the sidecar bound to the
current workspace, while ALGS import is one complete atomic replacement.

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
The separate canonical [`ALGC` V1 package](GRAPH-COMPONENT-V1.md) now provides
bounded public-terminal mappings and exact front-panel bindings around one
unchanged `ALGW`. Canonical [`ALGH` V3](GRAPH-HIERARCHY-V3.md) now binds scoped
component instances by exact package digest, rejects dependency cycles, bounds
recursive expansion, and deterministically flattens connector wiring to an
ordinary workspace using fresh monotonic IDs. Stable front-panel parameter IDs
also derive exact placeholder parameters, recurse through explicit parent panel
bindings, and retain per-root-occurrence values through complete-session
history. `ALFR` V1 now makes unowned Stream `InputControl` values executable in
the host UI. Its bounded timeline editor authors multiple strictly increasing
local-clock changes with exact schema-directed values, invalidates stale run
evidence on every draft edit, and leaves canonical authoring-session history
untouched. Hierarchy-resolved public Stream outputs now share a bounded exact
rational root-time cursor and select their latest exact trace sample at or
before it without creating new authority or saved state. Multi-value state
records, queue timeouts and additional policies, typed cases beyond the current
Boolean and exact-rational bindings, structured loops, general state-machine authoring tools,
capability-generated nodes
beyond stable Boolean inputs, multi-job prepare/start workflows and nested
component execution, workspace collaboration/conflict handling, broader
resource claims, general host implementation admission, measured WCET/deadline
analysis, physical HIL,
output and motion opcodes, live capability/configuration discovery,
capability-negotiated telemetry/trigger capture, and protocol-resource nodes
remain later M9 slices.
No arbitrary graph, component, or hierarchy document is sent to or interpreted
by firmware.
