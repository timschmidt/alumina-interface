# Capability-derived board explorer V1

The board explorer is a bounded diagnostic presentation of one complete
canonical `ALMCAP02` V2 document. It has no board-name branches in its core
model and grants no device operation merely because a resource is described.

## Identity and trust boundary

`alumina-capability::decode_board_capability` independently checks the complete
document header, exact length and SHA-256 identity, bounded nonempty UTF-8
strings, canonical enums/Booleans/options/routes/reserved bytes, typed resource
IDs, graph records, section counts, resource references, core ownership, safe
images, licensed visual records, normalized hotspot polygons, and the exact end
of the byte string. Its zero-allocation views borrow the caller's immutable
bytes. The interactive policy accepts at most:

- 4 MiB per complete document;
- 64 KiB per string;
- 4,096 records in any ordinary section;
- 32 visuals;
- 4,096 hotspots per visual; and
- 4,096 points per hotspot.

The decoder's SHA-256 is content identity, not authentication. The interface
must compare the returned complete identity with the capability identity from
its authenticated device/session before presenting a live device. The current
TinyBee demonstration instead uses the locally compiled, digest-verified board
package and labels the result as an offline reference.

## Description is not operation authority

`BoardExplorerSnapshot` owns a bounded UI copy of the decoded summary,
resources, aliases, visuals and hotspots. Each resource combines three facts
without conflating them:

1. its descriptive typed ID, exclusive core owner, reset/fault safe value and
   hazardous-output marker;
2. every canonical alias targeting that exact typed ID; and
3. only the graph class/access/support records explicitly published for that
   resource by the fixed firmware image.

A GPIO, UART, ADC, timer, shifted output, storage endpoint or radio appearing in
the descriptive inventory does not imply raw read, write, scheduling, capture,
test or graph access. V1 labels a resource as graph-readable only when the exact
graph section publishes the current `StableBooleanInput` operation. It exposes
no output operation.

## TinyBee offline reference

The visible explorer reconstructs the primary MKS TinyBee V1.x 8 MiB document
through the same 240-byte range encoder used by firmware. The 3,435-byte
document has SHA-256
`0e82513896e52e0a58fb92de9130c446d590bf649fbc22742209b2d04c8cb0a5`.
Its immutable view reports:

| Fact | Count |
| --- | ---: |
| descriptive typed resources | 62 |
| Service / Realtime owned | 21 / 41 |
| hazardous-output resources | 21 |
| graph-readable resources | 4 |
| aliases | 51 |
| buses / devices | 3 / 1 |
| flash regions / clocks | 0 / 2 |
| electrical constraints / interrupts | 9 / 4 |
| safe output images / HIL requirements | 1 / 8 |
| licensed visuals | 0 |

Search and mutually explicit filters expose all described, graph-readable,
graph-closed, hazardous, Service-owned and Realtime-owned resources. Selecting
a row shows the typed selector, aliases, owner, safe value, hazard state and
exact graph operation list. The four graph-readable resources remain GPIO22,
GPIO32, GPIO33 and GPIO35. For example, `axis.x.step` resolves to a hazardous
I2S shifted-output bit but remains graph-closed.

## Physical image gate

The current TinyBee capability deliberately contains no visual record. The UI
therefore draws no board silhouette, connector placement or hotspot and shows a
prominent missing-authority panel. A future physical overlay requires all of:

- an operator-owned, independently licensed photograph of the exact PCB
  revision;
- repository-relative asset path, MIME type and exact dimensions;
- SHA-256 over the exact raster bytes;
- SPDX license and attribution; and
- reviewed normalized polygons linking visible regions to typed resources.

Even after a record exists, the browser must fetch the raster, verify the
digest and dimensions, and only then draw its polygons. The existing
`visual.top-hotspots` hardware-in-the-loop gate remains open until the overlay
is reconciled against the physical fixture. Generated or vendor lookalike
imagery cannot stand in for that evidence.

## Closed claims

This checkpoint has no live capability acquisition, Wi-Fi traffic, telemetry,
sampling, trigger, oscilloscope, logic-analyzer, diagnostic lease, output test,
configuration mutation, arming or deployment path. It did not contact the
connected bare TinyBee. The panel is offline descriptive/editor state only.
