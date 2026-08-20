# Canonical graph diagnostic probes V2

`ALGP` V2 is a canonical sidecar for selecting graph outputs in plots and
logic-analyzer views. It also retains one optional replay-only Boolean edge
trigger and a bounded pre/post sample window. It binds authoring intent to one
exact `ALGW`; it does not grant firmware resource access, telemetry bandwidth,
device-trigger execution, storage, or deployment authority.

## Canonical sidecar

Every fixed-width integer is little-endian. The document contains, in order:

1. `ALGP`, version 2, and zero flags;
2. embedded maximum document bytes, probe count, samples per probe, and sample
   stride;
3. a monotonic document revision and next-probe identity cursor;
4. the SHA-256 identity of the exact external `ALGW`;
5. a fixed optional-trigger record; and
6. probe records sorted by stable probe identity.

Each probe record retains its ID, stable ASCII name, exact output node/port
endpoint, resolved `GraphTypeId`, maximum retained host values, and nonzero
event-ordinal decimation stride. V2 forbids duplicate IDs, names, or output
endpoints. Names are at most 64 bytes. The interactive policy admits at most
256 probes, one million retained values per probe, a one-million-value stride,
and 2 MiB of canonical sidecar bytes.

The fixed trigger record is four `u32` fields: edge kind, source probe ID,
pretrigger samples, and posttrigger samples. Kind zero disables the trigger and
requires the other canonical fields to re-encode as zero. Assigned kinds are
rising, falling, and either edge. An enabled trigger must name a retained
Boolean `Stream`; its requested pretrigger samples, one trigger sample, and
posttrigger samples must fit both the document policy and that probe's capture
ceiling.

The stride counts observed values; it is intentionally not a hidden physical
time unit. Edge detection compares consecutive values after that explicit
event-ordinal stride. Pre/post counts use those same retained values. Clock and
Stream semantics remain in the bound graph.

Replay first bounds outer bytes and embedded limits, resolves the exact
external workspace identity, reconstructs every source as an output port,
checks each retained value type and trigger source, rejects trailing bytes, and
re-encodes for byte-for-byte equality. Supplying another valid workspace fails
on identity before a probe can influence presentation.

## Exact replay resolution

`resolve_graph_probe_trigger` additionally requires the canonical simulation's
`ALGR` identity to equal the graph embedded by the bound workspace. It scans
only canonical trace records at the trigger probe's exact bound output, applies
the declared stride, and selects the first matching edge. A bounded ring
retains no more than the requested pretrigger samples plus the current sample
while waiting.

A match reports the exact graph clock, trigger tick and source sequence, first
and last available window ticks, retained pre/post counts, and whether each
requested side is complete. A finite replay with no edge reports `Waiting`;
absence of a configured trigger reports `Disabled`. Neither result is a device
capture operation.

## Bounded replay projection

`project_graph_probe_replay` is the shared HostExact boundary between canonical
simulation entries and plots. It rechecks workspace, graph, and fixed simulation
registry identities, resolves exact clock rates under the simulation's
independent root, and attaches an exact rational root-clock tick to every
retained sample while preserving the original clock/tick/sequence/value.

For each probe, event ordinal advances only over canonical trace entries at
that exact output. This includes caller-owned `ExternalInput` records at an
external-source output as well as modeled `NodeOutput` records; the original
trace-origin tag remains intact. Ordinal zero is retained, then every declared
`stride`th entry. With a matched trigger, decimated samples are admitted only
inside the trigger's first/last times after exact conversion to the shared root
clock.
With a disabled or waiting trigger, the complete finite replay is eligible.
In either case a trailing ring keeps no more than the probe's declared maximum
samples. Series remain in probe-ID order and samples remain in canonical
simulation order.

The caller additionally supplies an aggregate sample ceiling; the interactive
policy is 131,072 samples across all probes. Projection limits capacity before
allocation and rejects as soon as actual retained samples would exceed the
remaining aggregate budget. Zero limits, a foreign registry, a graph/workspace
substitution, unresolved clock analysis, or a sample on another independent
clock tree fails closed without partial output.

The current mixed-signal UI converts values directly from the projected exact
entries; changing retention or stride therefore changes the plotted points
rather than merely changing saved metadata. It displays the matched exact
root-time window and uses every retained sample's exact rational root tick as
its horizontal coordinate. Probes on different local rates within the shared
root can therefore coexist without conflating equal-looking local tick
integers. Local clock/tick identity remains visible in cursor labels. Every
nonempty projected Boolean or supported physical-scalar probe, including an
external-source output, is keyed by its canonical probe name and endpoint
rather than a fixed reference-series whitelist. The analog families are exact
rationals, exact closed measurement intervals, and signed or unsigned canonical
integer counts carrying a registered exact lattice quantum. They are grouped
in stable sample-type-ID order. Series with the identical registered type share
one certified-enclosure Y scale, while every distinct exact type receives its
own pane labeled with the schema's canonical type name, type ID, and unit
symbol. This prevents incompatible units or meanings from being overlaid while
retaining one exact root-time axis, trigger marker, and cursor across every
analog pane and the Boolean lanes. The display admits at most 32 distinct
analog type panes and fails explicitly above that bound; the ALGP sidecar and
replay remain unchanged.

An exact rational produces an outward binary64 enclosure only for display. A
measurement interval retains both exact endpoints and projects the lower
endpoint downward and upper endpoint upward. A canonical integer is multiplied
by its registered exact quantum before projection, while its original signed or
unsigned count remains in the cursor label. Reversed intervals, type/value
mismatches, missing unit metadata, and values without finite display
enclosures fail before painting. Text, structured, identity-bearing, and other
non-scalar values are not implicitly converted into analog data.

Only a named one-way enclosure maps exact root time into egui coordinates.
Pointer motion chooses the nearest displayed candidate but retains that
candidate's original `Rational`; it never converts a float coordinate back into
canonical time. The cursor candidates include retained sample times and the
exact matched-window anchors. At a shared cursor time, each series displays its
last retained value at or before that time, giving explicit sample-and-hold
semantics across rates. Non-finite projection or unordered time fails closed.

## Transactional editing

Adding and removing probes advances the sidecar revision and never reuses a
deleted identity. Removing the active trigger source atomically clears the
trigger. Installing, replacing, and clearing the trigger are transactional;
setting an identical trigger or clearing an already-disabled trigger is an
exact no-op. A workspace replacement can retain probes and trigger only when
every endpoint and exact value type survives. Rebinding to the identical
canonical workspace is also an exact no-op.

The UI can also transactionally replace one retained probe's name, maximum
sample count, and event-ordinal stride while preserving its stable identity,
source endpoint, and resolved value type. Names remain 1–64 ASCII bytes,
begin with a letter, and contain only letters, digits, `_`, `-`, or `.`; names
and source endpoints remain unique. Capture values remain nonzero and within
the sidecar's embedded one-million-sample/stride ceilings. Shrinking the active
trigger probe below its complete pre/current/post window rejects the entire
metadata edit. Reapplying identical metadata neither advances revision nor
changes canonical bytes.

Probe edits do not mutate the graph. The visible PID/interlock trace is filtered
by attached endpoints, so removing a probe removes only that plotted series.
Adding a valid output with no samples in the immutable reference `ALGT` still
records bounded authoring intent but invents no data.

The browser persists the current canonical `ALGP` together with its bound
`ALGW` in one versioned local-storage value. Pair restore replays both artifacts
before committing either, so a malformed sidecar or workspace-identity mismatch
falls back without partial state. Native and browser `.algp` exchange uses the
same 2 MiB byte admission and canonical replay boundary; import can replace
only the sidecar after proving the current `ALGW` identity, and importing
identical bytes is an exact no-op. Probe, capture-policy, and trigger identity
changes mark the pair dirty; canonical no-op edits do not. If a graph edit removes or retypes an
observed endpoint, the incompatible sidecar is visibly and atomically replaced
with an empty sidecar bound to the revised workspace rather than persisting
unbound probe intent. Ephemeral undo/redo retains complete canonical ALGW/ALGP
pairs, so undo restores the exact prior probes, trigger, revisions, and graph
binding even after such an invalidation; redo restores the exact revised pair.
Probe metadata/trigger edits and canonical ALGP imports are pair-history
operations too. Per-probe edit fields are transient UI state: unrelated
trigger or workspace-binding changes preserve a draft, while history
navigation and file/storage restore reset fields from the replayed canonical
sidecar.

The reference sidecar binds error, integral-prior, clamped-controller,
permit-gated-output, measurement-within-range, combined-permit, and independently
resampled external-permit endpoints. Its falling-edge trigger is bound to probe
5, `measurement-in-range`, with two pretrigger and two posttrigger samples. The
canonical 407-byte sidecar has SHA-256
`50955da7b4464a02f6e3eace1d51a3e9d08f56cdfa9c661259c33195b15ef223`.
The reference match is control tick 3 / sequence 3 with the complete exact
window from ticks 1 through 5.

The first four exact-rational series render as certified analog enclosures in
their registered-type pane. The last three Boolean series render as aligned
high/low logic-analyzer lanes. All panes use the trigger-selected exact time
window, per-probe decimation and retention, one cursor, and a distinct trigger
marker; none converts a retained value into a firmware command or physical
observation. The unchanged reference policy projects five samples for each of
seven probes (35 aggregate) inside ticks 1–5.

## Closed device claims

`ALGP` is not sent to the firmware graph interpreter—there is no arbitrary
interpreter—and it is not translated into a raw pin read or hardware trigger.
A future live probe path must separately authenticate target and
graph/configuration identities, negotiate bounded bandwidth and buffering from
capabilities, retain device cycle and loss/fault evidence, and enforce
safety/resource ownership. Until then the probe UI is a deterministic host-side
plot/authoring surface only.
