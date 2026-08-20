# Capability-derived board explorer V1

The board explorer is a bounded diagnostic presentation of one complete
canonical `ALMCAP04` V4 document. It has no board-name branches in its core
model and grants no device operation merely because a resource is described.

## Identity and trust boundary

`alumina-capability::decode_board_capability` independently checks the complete
document header, exact length and SHA-256 identity, bounded nonempty UTF-8
strings, canonical enums/Booleans/options/routes/reserved bytes, typed resource
IDs, graph, passive-diagnostic, and digital-capture records, section counts,
resource references, core ownership, safe images, licensed visual records,
normalized hotspot polygons, and the exact end of the byte string. Its
zero-allocation views borrow the caller's immutable
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
paired diagnostic demonstration uses the locally compiled, digest-verified
host simulator package and labels every value/source as simulated. The physical
TinyBee remains a separate capture-absent identity.

## Description is not operation authority

`BoardExplorerSnapshot` owns a bounded UI copy of the decoded summary,
resources, aliases, visual descriptors and hotspots. Raster bytes are a
separate immutable transfer: the capability commits their SHA-256, media type,
dimensions, license and attribution but does not inline them. Each resource
combines five facts
without conflating them:

1. its descriptive typed ID, exclusive core owner, reset/fault safe value and
   hazardous-output marker;
2. every canonical alias targeting that exact typed ID;
3. only passive semantic observation/support records explicitly published for
   that resource, independent of graph access; and
4. only the digital-acquisition source/support record explicitly published for
   that resource; and
5. only the graph class/access/support records explicitly published for that
   resource by the fixed firmware image.

A GPIO, UART, ADC, timer, shifted output, storage endpoint or radio appearing in
the descriptive inventory does not imply raw read, write, scheduling, capture,
test or graph access. The explorer labels a resource passively observable only
when `ALMDOV01` publishes an implemented record under an implemented provider,
digitally capturable only when `ALMDCP01` publishes an exact source record and
fixed acquisition budget, and graph-readable only when the exact
graph section publishes the current `StableBooleanInput` operation. It exposes
no output operation.

## TinyBee simulator diagnostic reference

The visible paired diagnostic explorer reconstructs the distinct host-only
`sim-mks-tinybee-v1` document through the same 240-byte range encoder used by
firmware. The 4,028-byte
document has SHA-256
`218cc758f430f8897f8c7dbcda6c1af2077fae5fc0062cce2fe7cb49a066aa79`.
Its immutable view reports:

| Fact | Count |
| --- | ---: |
| descriptive typed resources | 62 |
| Service / Realtime owned | 21 / 41 |
| hazardous-output resources | 21 |
| passively observable resources | 4 |
| digitally capturable resources | 4 |
| graph-readable resources | 4 |
| aliases | 51 |
| buses / devices | 3 / 1 |
| flash regions / clocks | 0 / 2 |
| electrical constraints / interrupts | 9 / 4 |
| safe output images / HIL requirements | 1 / 8 |
| licensed visuals / reviewed hotspots | 1 / 4 |

Search and mutually explicit filters expose all described, passively
observable, diagnostic-closed, digitally capturable, capture-closed,
graph-readable, graph-closed, hazardous,
Service-owned and Realtime-owned resources. Selecting a row shows the typed
selector, aliases, owner, safe value, hazard state, exact passive observation,
capture source, and graph operation lists. The model's four observable and
capturable resources are
GPIO22, GPIO32, GPIO33 and GPIO35; the graph palette independently happens to
contain the same selectors today. That coincidence grants no cross-catalog
authority. The overview catalog also reports schema 1,
176/432-byte request/event budgets, a 100,000 µs cadence, and a 500,000 µs
freshness ceiling. The capture catalog separately reports simulated sources,
schema 1, four channels, 64 transitions, 208/2,048/168-byte
configure/record/chunk budgets, immediate trigger, zero pretrigger, and
2,000,000/30,000,000 µs duration/arm limits. The sole
`simulated-topology` visual is a 109-byte, 40 x 20 PNG with SHA-256
`79fcf464e343ffc8f33cdec96a8e2fd87da886c57dd4be080898a5fc59b52412`.
It is an Alumina-authored CC0 four-quadrant diagnostic fixture, explicitly
attributed as not being a PCB photograph. Its four polygons link GPIO22,
GPIO32, GPIO33 and GPIO35 to the corresponding quadrants. For example,
`axis.x.step`
resolves to a hazardous I2S shifted-output bit but remains diagnostic-closed,
capture-closed, and graph-closed.

## Physical image gate

The synthetic simulator visual exercises acquisition, decoding, rendering,
picking and live typed-resource linkage without making a physical claim. The
current physical TinyBee capability deliberately contains no visual record.
For that identity the UI draws no board silhouette, connector placement or
hotspot and shows a prominent missing-authority panel. A future physical
overlay requires all of:

- an operator-owned, independently licensed photograph of the exact PCB
  revision;
- repository-relative asset path, MIME type and exact dimensions;
- SHA-256 over the exact raster bytes;
- SPDX license and attribution; and
- reviewed normalized polygons linking visible regions to typed resources.

After a complete capability is authenticated, the dedicated worker downloads
each declared raster with `CapabilityVisualGet` in contiguous ranges, pins the
capability and asset digests plus the first declared byte length, and verifies
the complete SHA-256 before publishing bytes. Worker schema V13 revalidates
that private transfer after JSON decoding. The rendering realm rebinds it to
the current connection generation and capability, verifies SHA-256 again,
decodes only PNG at this checkpoint, requires the exact declared dimensions,
and enforces 64 MiB per unique decoded RGBA image plus 128 MiB across one
capability before only then drawing normalized polygons. The PNG decoder also
receives strict declared width/height limits and a 128 MiB transient allocation
ceiling. A generation, boot, identity,
capability or disconnect transition removes the stale asset. The existing
`visual.top-hotspots` hardware-in-the-loop gate remains open until the overlay
is reconciled against the physical fixture. Generated or vendor lookalike
imagery cannot stand in for that evidence.

## Closed claims

The simulator path has authenticated localhost capability and visual
acquisition plus simulated low-rate telemetry and retained capture. It has no
physical Wi-Fi, sampling, trigger, oscilloscope, logic-analyzer, diagnostic
lease, output test, configuration mutation, arming or deployment claim. This
work did not contact the connected bare TinyBee or workstation WLAN. Hotspot
selection remains descriptive UI state and grants no I/O operation.
