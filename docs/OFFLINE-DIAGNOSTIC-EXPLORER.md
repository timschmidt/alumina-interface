# Offline diagnostic explorer checkpoint

The board explorer now consumes two canonical byte records rather than UI-only
sample objects:

- `ALMOVW01`, a bounded resource overview; and
- `ALMDIG01`, a bounded triggered digital edge capture.

`alumina-interface-core` independently decodes the records, requires identical
device/boot/config/clock context, requires both complete capability identities
to match the already decoded board package, and rejects every typed resource
not present in that board package. It allocates owned presentation vectors only
after these checks pass.

The current visible data comes from
`alumina-sim::diagnostics::tinybee_diagnostic_fixture`. It is explicitly marked
simulated at both document and sample/acquisition layers. It opens no network,
serial port, GPIO, board handle, command path, or diagnostic lease.

The TinyBee ledger shows explicit overview values, provenance, quality, and age
only for the four records actually present. The selected-resource card, ledger,
and digital lanes use the same `ResourceId`. The four-lane plot preserves its
integer cycle interval, trigger event, retained edge order, buffer capacity,
and quality flags; hover/click is a lossy screen projection used only for a
cursor and selection. It cannot alter canonical evidence or produce a command.

The existing physical-view gate remains unchanged: no board outline, connector
placement, or hotspot is drawn until a licensed revision-specific image and
reviewed polygons exist in the capability package.

Firmware wire details, tests, and open physical/network gates are recorded in
[`../aluminafw/docs/DIAGNOSTICS.md`](../../aluminafw/docs/DIAGNOSTICS.md) and the
[`M9 offline diagnostic evidence`](../../aluminafw/docs/evidence/M9-OFFLINE-DIAGNOSTIC-EXPLORER.md).
