# Canonical graph diagnostic probes V1

`ALGP` V1 is a canonical sidecar for selecting graph outputs in plots,
logic-analyzer views, and later telemetry captures. It binds authoring intent
to one exact `ALGW`; it does not grant firmware resource access, telemetry
bandwidth, trigger execution, storage, or deployment authority.

## Canonical sidecar

Every fixed-width integer is little-endian. The document contains, in order:

1. `ALGP`, version 1, and zero flags;
2. embedded maximum document bytes, probe count, samples per probe, and sample
   stride;
3. a monotonic document revision and next-probe identity cursor;
4. the SHA-256 identity of the exact external `ALGW`; and
5. probe records sorted by stable probe identity.

Each record retains its ID, stable ASCII name, exact output node/port endpoint,
resolved `GraphTypeId`, maximum retained host values, and nonzero event-ordinal
decimation stride. V1 forbids duplicate IDs, names, or output endpoints. Names
are at most 64 bytes. The interactive policy admits at most 256 probes, one
million retained values per probe, a one-million-value stride, and 2 MiB of
canonical sidecar bytes.

The stride counts observed values; it is intentionally not a hidden physical
time unit. Clock and Stream semantics remain in the bound graph. Later trigger
and hardware-capture policies require new explicit typed authority rather than
overloading this field.

Replay first bounds outer bytes and embedded limits, resolves the exact
external workspace identity, reconstructs every source as an output port,
checks the retained value type, rejects trailing bytes, and re-encodes for
byte-for-byte equality. Supplying another valid workspace fails on identity
before a probe can influence presentation.

## Transactional editing

Adding and removing probes advances the sidecar revision and never reuses a
deleted identity. A workspace replacement can retain probes only when every
endpoint and exact type survives; otherwise the operation returns no candidate.
The UI treats such failure as sidecar detachment without weakening or rejecting
the underlying `ALGW` draft.

Probe edits do not mutate the graph. The visible PID/interlock trace is filtered
by attached probe endpoints, so removing a probe removes only that plotted
series. Adding a valid output with no samples in the immutable reference `ALGT`
still records the bounded authoring intent but invents no data.

The reference sidecar binds error, integral-prior, clamped-controller, and
permit-gated-output endpoints. It is 257 bytes with SHA-256
`3bbd8ff29e118f3f0a37885adf13e263252ecc01148d50eb88058be1a1b42651`.

## Closed runtime claims

`ALGP` is not sent to the firmware graph interpreter—there is no arbitrary
interpreter—and it is not translated into a raw pin read. A future live probe
path must separately authenticate the target and graph/configuration identity,
negotiate bounded bandwidth and buffering from capabilities, retain device
cycle and loss/fault evidence, and enforce safety/resource ownership. Until
then the probe UI is a deterministic host-side plot/authoring surface only.
