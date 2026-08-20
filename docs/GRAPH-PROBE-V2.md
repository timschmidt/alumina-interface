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
only the trigger probe's canonical node-output records, applies the declared
stride, and selects the first matching edge. A bounded ring retains no more
than the requested pretrigger samples plus the current sample while waiting.

A match reports the exact graph clock, trigger tick and source sequence, first
and last available window ticks, retained pre/post counts, and whether each
requested side is complete. A finite replay with no edge reports `Waiting`;
absence of a configured trigger reports `Disabled`. Neither result is a device
capture operation.

## Transactional editing

Adding and removing probes advances the sidecar revision and never reuses a
deleted identity. Removing the active trigger source atomically clears the
trigger. Installing, replacing, and clearing the trigger are transactional;
setting an identical trigger or clearing an already-disabled trigger is an
exact no-op. A workspace replacement can retain probes and trigger only when
every endpoint and exact value type survives. Rebinding to the identical
canonical workspace is also an exact no-op.

Probe edits do not mutate the graph. The visible PID/interlock trace is filtered
by attached endpoints, so removing a probe removes only that plotted series.
Adding a valid output with no samples in the immutable reference `ALGT` still
records bounded authoring intent but invents no data.

The reference sidecar binds error, integral-prior, clamped-controller,
permit-gated-output, measurement-within-range, combined-permit, and independently
resampled external-permit endpoints. Its falling-edge trigger is bound to probe
5, `measurement-in-range`, with two pretrigger and two posttrigger samples. The
canonical 407-byte sidecar has SHA-256
`50955da7b4464a02f6e3eace1d51a3e9d08f56cdfa9c661259c33195b15ef223`.
The reference match is control tick 3 / sequence 3 with the complete exact
window from ticks 1 through 5.

The first four exact-rational series render as certified analog enclosures. The
last three Boolean series render as aligned high/low logic-analyzer lanes. Both
views use the trigger-selected exact time window, one cursor, and a distinct
trigger marker; neither converts a retained value into a firmware command or
physical observation.

## Closed device claims

`ALGP` is not sent to the firmware graph interpreter—there is no arbitrary
interpreter—and it is not translated into a raw pin read or hardware trigger.
A future live probe path must separately authenticate target and
graph/configuration identities, negotiate bounded bandwidth and buffering from
capabilities, retain device cycle and loss/fault evidence, and enforce
safety/resource ownership. Until then the probe UI is a deterministic host-side
plot/authoring surface only.
