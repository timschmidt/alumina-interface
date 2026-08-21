# Cache-derived graph job catalog V1

`JobHandle` is an inert graph value containing one stable participant MCU,
one shared global-job digest, and one participant-local partition digest. It is
not a path, a mutable job name, a deployment token, a resource lease, or start
authority. The V1 editor therefore never parses those three identities from
text.

## Authority join

Two independent inputs are required before the editor can offer a handle:

1. `CanonicalGlobalJob2` retains the canonical `ALMJMF02` manifest, every
   participant record, every independently replayed partition package, and
   their exact `PublishedObject` identities.
2. `ParticipantCacheReady` is emitted by the Wi-Fi cache-delivery coordinator
   only after that participant's partition upload machine and its copy of the
   shared global-manifest upload machine both reach `Complete`.

`derive_graph_cached_job_catalog_from_ready` converts those crate-private
readiness tokens into data-only publication evidence and invokes the
window-free core derivation. Core derivation is still fail-closed: it validates
the caller bound, requires exactly one evidence item per participant, sorts by
stable `DeviceId`, rejects duplicates, and compares the complete partition and
global-manifest publications exactly with the canonical job. The package and
manifest participant records must also agree on device and partition identity.
Discovery order, upload transaction IDs, labels, and UI position have no role.

The resulting entries retain the complete canonical `MachineJobParticipant`
record. The UI can therefore report the exact capability/configuration
digests, partition size and block count, timer frequency, execution kind, and
other CAM facts needed for later precision and schedule decisions without
inventing them from a board name.

`GraphCachedJobPublicationEvidence` is deliberately documented as data-only.
Its public constructor validates identity shape and object kind, but does not
authenticate a transport. Applications should use the readiness-token adapter;
the exact canonical comparison remains mandatory in either case.

## Transactional graph selection

`GraphCachedJobCatalogEntry::typed_value` constructs a root `JobHandle` only for
a registered `TypeKind::JobHandle`. `select_graph_cached_job_handle` additionally
accepts a borrowed bounded value path. An empty path selects that root; a
nonempty path may use only stable record-field IDs, retained array indices, and
explicit existing option/result branches. It never resolves a display name or
constructs an absent branch. The selector then requires:

- an existing node and parameter;
- an exact schema-resolved `JobHandle` leaf;
- exact membership of the current handle in the same catalog, closing raw,
  stale, and foreign values;
- a different, in-range catalog choice; and
- complete structural mutation, audited draft semantic analysis, and
  canonical `ALGW` encoding on a clone before commit.

The complete root parameter is reconstructed and revalidated after one leaf is
replaced. The operation preserves every sibling value, node identity, label,
domain, ports, canvas placement, and both allocation cursors. An exact no-op
creates no revision. Unknown fields, out-of-range indices, inactive branches,
wrong leaf types, or structurally valid raw handles fail before mutation.
Unlike a physical resource selector, it deliberately permits multiple nodes to
retain the same handle: an immutable reference is data, and uniqueness would
falsely suggest command ownership. A later prepare/start node or coordinator
must enforce its own ownership, safety, cache, clock, and deterministic-start
authority.

## Visible offline workflow

The Control workspace includes a separate offline cached-job authoring proof.
It drives the deterministic two-MCU cache simulation through the real
partition-then-manifest delivery state machines, derives the catalog from the
resulting readiness tokens, and builds a minimal reviewed HostExact graph whose
only parameter is an exact record containing a primary job, an optional
fallback, and a bounded mirror array. The operator can select one of those
three explicit leaves, select a catalog participant, add another inert
reference set, rebind the selected leaf, and undo or redo. The UI displays the
stable named projection of the fixed path, but there is no editable path,
digest, device ID, partition ID, file path, or command field.

The panel visibly reports that it is simulated and non-executing. It shows the
global job and participant-set prefixes, canonical graph identity and byte
length, exact participant cache facts, and bounded history state. Every undo
or redo target replays canonical `ALGW` bytes and reruns both semantic and
catalog-membership admission before it replaces current state.

## Origin-local persistence

Application persistence is now one versioned `algwb1:` bundle containing
lowercase-hex control `ALGW`, bound `ALGP`, and catalog-bound cached-job `ALGW`
sections. This is a greenfield replacement, not a compatibility envelope. Each
section has an independent 2 MiB ceiling. Restore replays and admits all three
artifacts first; the cached-job graph must still resolve every handle against
the catalog derived at this startup. Only then are any in-memory documents
replaced. Missing, extra, uppercase, oversized, noncanonical, mismatched-probe,
raw-handle at any nested leaf, stale-handle, or foreign-job input rejects the
complete bundle.
History remains ephemeral and is cleared after restore.

## Closed claims

This boundary does not prove a live authenticated session merely from a
`PublishedObject`, make simulated cache evidence live, upload a partition,
prepare or install a schedule, synchronize clocks, arm an output, start or stop
motion, or authorize firmware execution. It adds no firmware operation and
does not contact hardware or WLAN. The implementation is independently authored
under MIT and introduces no dependency or GPL-family code.
