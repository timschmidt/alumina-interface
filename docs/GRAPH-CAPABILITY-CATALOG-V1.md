# Capability-derived graph node catalog V1

The first physical-resource palette is derived from two independent
authorities. A complete canonical board capability document says what one
firmware image advertises. A reviewed `GraphDeploymentRegistry` says which
opaque graph kinds have fixed semantics, opcode bindings, schedule clocks, and
WCET. `derive_graph_capability_node_catalog` intersects them; neither source can
create an editor node alone.

## Fail-closed derivation

The caller supplies:

- the complete bounded `ALMCAP03` document;
- one nonzero device, capability, and active-configuration target tuple;
- the reviewed semantic/deployment registry for the application version; and
- explicit document-byte and derived-entry limits.

The catalog verifies canonical bytes and content identity. Authentication of
the live device/session remains the caller's protocol responsibility; a digest
is not an authenticator.

The decoder hashes the complete document and requires that digest to equal the
target capability identity. For the current V1 operation, the deployment
registry must bind a Realtime node to `StableBooleanInput`; its sole parameter
must be a resource-handle type, and its output must be the reviewed Boolean
Stream shape. The image must independently advertise opcode 4 in Realtime with
the same nonzero resource class, `StableBooleanInput` access, and at least
`Compiles` support. Only resource records with the same class/access/support
survive the intersection.

Every resulting `GraphCapabilityNodeEntry` owns a complete
`GraphNodePrototype`. Its resource parameter contains the exact target
`DeviceId`, capability/board-package digest, resource class, and canonical
four-byte typed selector. Its placement is Realtime on that same device. The
parameter name, type, ports, and node kind come from the reviewed schema rather
than the board document. Entries are sorted by kind/version and canonical typed
resource encoding, not discovery order.

Inserting the prototype into an `ALGW` still performs complete structural
validation. The resulting graph must later pass semantic analysis, fixed
implementation admission, schedule/WCET and arena proof, target/configuration
binding, and firmware package replay. The catalog is not deployment authority.

## TinyBee offline proof

The browser/native UI builds the exact MKS TinyBee V1 8 MiB capability bytes
from the sibling `board-mks-tinybee` package. Its 3,531-byte document has
SHA-256
`27dcdd9ea4a1f9fcb1a4aeefb34984a4e4a0ca146c660f669bf632f98cac74af`.
The reviewed intersection exposes exactly four read-only resources, in
canonical order:

1. GPIO22;
2. GPIO32;
3. GPIO33; and
4. GPIO35.

The visible target-I/O surface uses a separate Realtime workspace with an
explicit 240 MHz reference device-cycle clock and a derived 1 kHz input clock.
Each catalog choice can create at most one concrete resource node in that
draft. The HostExact PID/interlock workspace has a different type/clock context
and is never polluted with a physical handle.

The proof deliberately uses a conspicuous offline reference `DeviceId` and
configuration digest. It cannot identify or deploy to the connected TinyBee.
The production path must replace all three target identities with values from
an authenticated live session. Compiling a reference board package into the UI
does not make its nominal identity a live-device identity.

TinyBee also describes ADCs, UARTs, timers, shifted outputs, storage, and other
GPIOs in its broader board inventory. They are not entries in the authenticated
graph-execution resource palette and remain visibly closed. The catalog does
not infer raw GPIO access from pin numbers or convert a descriptive board
resource into graph authority. A later peripheral must first add a typed access
enum, fixed graph opcode, board capability record, reviewed host binding,
firmware admission/execution path, and appropriate safety evidence.

## Current verification

Tests reconstruct the complete TinyBee capability document through its bounded
range API, prove the exact four-entry order, inspect every target-bound resource
handle, insert all four prototypes transactionally into the matching context,
and rerun audited draft analysis. A wrong capability digest, over-limit
document, and entry-count ceiling fail without returning a partial catalog.
UI tests add all four concrete nodes, reject a duplicate selection without
changing canonical bytes, and reset the separate draft.

This is offline functional evidence. No Wi-Fi interface, connected board,
motor, output, or analyzer was contacted or driven.
