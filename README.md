# Alumina Interface

Alumina Interface is the greenfield browser/WASM authority for Alumina CAD/CAM,
machine configuration, job compilation, control, and diagnostics. The current
checkpoint establishes the exact geometry and protocol foundations; it does not
preserve the routes, graph files, renderer, or device model of the earlier
prototype.

## Current baseline

- `crates/alumina-interface-core` is window-free and owns exact design/CAM
  values, bounded unit-bearing measurements, canonical integer machine values,
  explicit one-way display projections, and the checked Hypercurve-to-Hyperpath
  metric boundary.
- `crates/alumina-interface-client` owns the headless native protocol client,
  deterministic simulator transport, origin-bound HMAC V2 session, strict boot
  challenge decoder, retry-safe content-addressed upload reconciliation,
  context/digest-bound telemetry and capture reconciliation, and a WASM `fetch`
  adapter. Its versioned UI/worker contract carries bounded
  commands and redacted clock snapshots; browser and native paths share
  canonical frame and response-proof validation.
- The native/browser shell composes `ExactScene` and `ExactCamera` values and
  uploads them through Hypergraphics. It contains no application-owned vertex,
  normal, grid, camera-matrix, or primitive-float geometry pipeline.
- CSGRS and every Hyper dependency resolve to sibling repositories in the
  shared workspace. There is no crates.io CSGRS fallback.
- CSGRS builds native `TriangleMesh` geometry. Hypergraphics performs checked
  mesh expansion and certified Hypercurve chord subdivision into exact scene
  vertices, and owns the only f64/f32 GPU boundary.
- The baseline renders an exact line/arc/cubic source path with retained chord
  evidence. Exact lines and circular arcs promote losslessly to Hyperpath; a
  general cubic fails with a typed metric blocker instead of borrowing its
  display chords. The machine compiler can instead admit that cubic through a
  distinct bounded pointwise certificate over Hypercurve's exact de Casteljau
  subcurves. The retained
  line/semicircle fixture certifies the symbolic path length `4 + 2*pi` through
  Hyperpath and Hypersolve.
- A separate motion-specific compiler certifies source chords, rounds every
  coordinate and cumulative time with Hyperreal's certified integer boundary,
  replays half-lattice/half-tick bounds through Hyperlimit, and emits the real
  `alumina-machine-ir::ExecutionSegment` type. The deterministic fixture uses
  80 steps/mm, 1 MHz ticks, 10 mm/s, and a `1/1024 mm` source-chord budget.
- The production-oriented line/arc/cubic path derives a two-axis dynamics profile
  directly from validated canonical Configuration V6: exact resource and
  transmission facts, uncertainty intervals, travel, pulse-rate ceiling,
  velocity/acceleration/jerk/following limits, device clock, and backend output
  quantum. A machine-wide resolution certificate composes source and controller
  allocations with endpoint, DDA, calibration, following, and one-output-quantum
  position bounds before scheduling. Intentional replay-proved time dilation is
  reported separately rather than mislabeled as spatial error.
- For an all-line metric route, Hyperpath now projects every dense axis's exact
  velocity, acceleration, and jerk limits through the retained constant
  `|dq_i/ds|`. It selects route-wide scalar minima, retains the first exact
  feed/acceleration/jerk bottleneck, and replays every span/axis inequality plus
  those equalities through Hypersolve. A 3-4-5 diagonal regression obtains the
  exact `5/4` utilization factor. At the exact continuous electrical ceiling,
  factor one is rejected by production preflight; one-sided interval
  quantization and a bounded `1/4096` rational-factor search select exactly
  `4158/4096`, prove the immediately smaller candidate fails, and lower to the
  expected terminal steps. Mixed curved routes keep the earlier conservative
  direction-independent envelope until their higher-derivative terms are
  certified.
- The machine compiler preserves lines/arcs losslessly and reduces a polynomial
  cubic only under an exact pointwise positional certificate, a 16,384-element
  bound, and a depth bound. Hyperpath now derives speed nodes with exact
  squared-speed forward/reverse reachability, tangent classification, retained
  radii, and caller-owned ceilings, then partitions positive nodes at exact
  stops and halves each component until every monotonic jerk transition
  independently certifies. Alumina permits positive caller ceilings only at
  lossless exact line-to-line joins; Hyperpath's tangent predicates still
  require G1 continuity. Entry, exit, arcs, corners, reversals, and every cubic
  chord boundary remain exact stops. The browser evaluates this dedicated
  metric path—not renderer chords—while subdividing those phases under the
  exact `A*dt²/8` interpolation bound. It rounds only at the configured
  step/output lattices, rejects any phase that would exceed the caller-owned
  131,072-point interactive allocation before fallible reservation, then
  rounds each ideal interval upward to the exact output quantum and searches a
  caller-bounded rational time-dilation lattice through complete production
  replays. It replays every emitted segment through the
  allocation-free production stepper executor's pulse, rate, direction,
  enable, output-grid, continuity, overflow, and terminal checks.
- Phase construction reads the selected lookahead boundary nodes. A zero/zero
  element retains the four-phase rest-to-rest schedule. An element with at
  least one positive boundary uses Hyperpath's exact two-phase monotonic
  transition with zero endpoint acceleration and independent construction and
  kinematic replay. Dedicated exact-line tests lower positive G1 motion through
  production executor preflight; curvature-bearing joins remain disabled. A
  projected continuous limit which lands exactly on an electrical pulse ceiling
  now receives only replay-proved timer-lattice headroom: factor one and the
  immediately smaller factor-grid candidate remain retained failures, while an
  exhausted caller ceiling still fails closed. No arbitrary floating-point
  margin is inserted.
- Before scheduling, Hypercurve's complete native source bounding box is
  compared exactly with the uncertainty-reduced usable travel from the same
  configuration. This catches arc extrema between interpolation samples and
  fails before lowering when any exact boundary lies outside travel. Every
  rounded canonical point is checked again, so an outward half-step cannot
  escape the usable interval.
- The machine-bound program packages only into a partition with identical
  capability/configuration digests. The resulting cached bytes run through an
  event-level `RealtimeJob`/`CachedStepperExecutor` simulator, and canonical
  `ALMEVD03` evidence independently binds exact-rational source, metric path,
  source-to-motion certificate, physical/error policy, every retained exact
  affine/lookahead/jerk certification decision, complete timer-search and
  lowering evidence, executor results, and content-addressed partition
  identities. Planner and lowering subtranscripts are domain-separated,
  incrementally hashed, and committed by both SHA-256 and byte length. The same
  representative test binary compiles for WASM. This remains software evidence,
  not TinyBee timing or motion qualification.
- Same-grid multi-MCU jobs now select one exact factor before immutable cache
  publication. Participants are sorted by stable device identity and must share
  the exact ideal event grid, timer, and output quantum; every candidate is a
  complete production-preflight round over every MCU. The accepted streams are
  replayed against their exact point carriers, independently partitioned, and
  committed by a 104-byte `ALMSYN01` record over a streamed `ALMSRT01`
  transcript containing the full search trace, derivation identities, final
  IR, and partition identities. The compiler derives global duration and
  synchronization evidence before emitting `ALMJMF02`; mixed-clock grids fail
  closed.
- The shell now opens an offline Machine/CAM inspector by default. One
  canonical `ALMCFG06` TinyBee fixture drives exact axis/transmission facts,
  travel proof, resolution-budget decomposition, retained-path diagnostic
  projection, exact acceleration/jerk-feasible schedule tables, canonical
  points and segments, production executor preflight, SD-cache identities, event-level
  replay, `ALMEVD03` evidence, and a two-participant evidence-bound shared cache
  and global manifest. Native and browser file exchange can replace
  configuration state only after the entire chain reconstructs successfully;
  evidence imports must equal a fresh reconstruction byte for byte. This view
  initiates no device connection and has no arming or output authority. See
  [`docs/OFFLINE-MACHINE-CAM.md`](docs/OFFLINE-MACHINE-CAM.md).
- The cubic motion contract, bounded failure modes, exact-stop rationale, error
  composition, and evidence domains are documented in
  [`docs/CERTIFIED-BEZIER-MOTION.md`](docs/CERTIFIED-BEZIER-MOTION.md).
- Exact Hyperreal servo recurrences now project once to Q31.32/Q2.30 with
  retained certified error bounds, split at discrete extrema and
  configuration-derived horizons, replay through the firmware validator, and
  package as generic one-to-four-axis `ALMBLK03` kind-3 cached jobs. See
  [`docs/EXACT-SERVO-MOTION.md`](docs/EXACT-SERVO-MOTION.md).
- The same workspace has a bounded UI-only CNC geometry adapter for one
  connected XY line/explicit-IJ-arc path. It parses every decimal into an exact
  rational, requires explicit unit/plane/endpoint/arc-centre modal state,
  rejects every process or unsupported word, and transactionally rebuilds the
  complete schedule/cache/replay chain. Raw text has a separate provenance
  digest and never becomes canonical or firmware input. See
  [`docs/EXACT-CNC-GEOMETRY-IMPORT.md`](docs/EXACT-CNC-GEOMETRY-IMPORT.md).
- Canonical segments are deterministically partitioned using the firmware's
  queried record capacity and caller-owned horizon limits. Every chained
  512-byte block is independently replayed before `alumina-storage` creates the
  real resumable upload, chunk-manifest, publication, and later boot-local
  `alumina-job::JobDescriptor` bytes.
- Owned per-MCU artifacts are sorted by stable device identity into the real
  canonical global job manifest. Its exact content and participant-set digests
  bind directly to firmware schedule commits, and the same manifest can use an
  independent resumable upload transaction on every MCU.
- Per-participant delivery now binds both exact publications before I/O,
  reconciles the executable partition first, then the identical global
  manifest, and returns to `StorageInspect` after any ambiguous fetch. Browser
  fetch omits ambient credentials, rejects redirects, disables caching, and
  binds request/response proofs to the actual document origin.
- The browser shell creates one dedicated module worker that owns each device's
  HMAC secret, HTTP session, causal clock model, automatic retry cadence, and a
  bounded 64-observation history. It also owns passive runtime-health polling,
  retains independently validated queue/stack evidence and health-specific
  failures, retry-safe bounded board-capability acquisition, capability-selected
  live telemetry, bounded retained waveform capture, and capability-bound
  immutable visual acquisition. Strict schema-v13
  snapshots expose only redacted progress and immutable identity; complete
  canonical capability, telemetry, and capture documents cross to the UI only
  after independent validation. The UI can add, probe, disconnect, and request
  passive input capture without receiving credentials or raw-pin authority.
- That worker exchanges production-format authenticated heartbeat traffic with
  the deterministic host MCU fixture and recovers from response loss, a finite
  outage, and reboot while conservatively rejecting excessive delay.
- The headless multi-MCU coordinator now retains each authenticated first-output
  observation, rejects later evidence regression, exactly inverts its
  boot-scoped device-cycle interval into conservative browser monotonic bounds,
  and preserves simulator/peripheral/software authority. The repeatable
  two-device flow proves each known simulated edge is contained and exposes
  outer edge-spread and shared-epoch-error bounds in the diagnostic UI.
- The window-free core now owns the first greenfield typed graph document:
  exact unit/type registries, bounded typed literals, explicit clocks and
  HostExact/Service/Realtime domains, opaque versioned nodes, typed ports and
  wires, and a canonical bounded `ALGR` V1 codec. Untrusted loads enforce an
  independent admission policy, rebuild through the validators, require exact
  byte replay, and derive a SHA-256 graph identity. This is structural graph
  authority only; it never becomes an arbitrary firmware graph interpreter.
- A separately bounded node registry resolves opaque kinds only when exact
  port/parameter shapes, allowed domain families, complete current-tick
  feedthrough, and optional read-before-write state are declared against the
  document's exact type/clock context. Iterative port-level analysis accepts
  deliberate delayed feedback and returns exact wire/feedthrough witnesses for
  forbidden combinational cycles; it emits no executable implementation.
- Checked recursive storage analysis proves canonical maximum bytes for every
  literal or runtime payload/sample type and rejects state storage smaller than
  its complete exact value domain. It does not claim a firmware runtime layout.
- Required/optional input contracts distinguish same-owner synchronous slots
  from bounded event/stream queues with explicit full behavior. Exact reports
  include timestamp/sequence envelopes and reject scalar cross-domain sharing,
  stream over-capacity, and per-input/total allocation overflow.
- Cross-clock Stream feedthrough requires an audited latest-at-or-before
  transition. Exact clock resolution proves a shared tick-zero root, the
  smallest rational schedule pattern, minimum input capacity, and separately
  bounded held-sample state; implicit or independent-root transitions reject.
- A separate implementation registry admits eleven reviewed `HostExact`
  simulation behaviors: external Stream source, audited latest-at-or-before
  transition, Stream sink, exact add/subtract/scale/clamp, explicit
  inclusive-range predicates, Boolean conjunction, read-before-write unit
  delay, and a fail-safe Boolean permit gate. A visible
  multi-rate discrete PID/interlock fixture composes those primitives without
  hidden controller state. The bounded simulator uses exact rational clock
  time and unit scales, orders every coincident source tick first, and produces
  the same canonical result regardless of caller sample order.
- The native and browser shells construct that same fallible core fixture and
  open a bounded workspace by default: semantic current-tick layers, explicit
  feedback routes, typed ports, exact parameters/state, and seven mixed-signal
  control traces: four exact-rational signals and three Boolean interlock lanes.
  Canonical `ALGW` V1 embeds the unchanged `ALGR` plus integer canvas positions
  and monotonic ID cursors. Its 13-entry fixed-schema palette supports
  monotonic node creation, atomic node/incident-wire deletion, node moves,
  typed wire edits, and bounded schema-directed exact scalar/composite literal
  editing. The node inspector also edits bounded canonical UTF-8 labels and
  concrete execution placement: domain families come from the audited node
  schema, while device identities come only from clocks or placements already
  present in the graph. There is no raw device-ID field, and complete audited
  analysis runs before commit. Deterministic quoted text, hexadecimal bytes,
  arrays, records,
  options, and results round-trip through the window-free core; resource/job
  handles remain selector-bound rather than text-authorized. A separate
  [cache-derived job catalog](docs/GRAPH-CACHED-JOB-CATALOG-V1.md) joins the
  canonical global CAM manifest to exact partition-plus-manifest cache-ready
  observations, retains the precision-relevant participant record, and offers
  only those inert participant-local references in an offline graph selector.
  Its bounded value paths address stable record fields, existing option/result
  branches, and retained array elements, so the visible proof can rebind exact
  primary, optional fallback, or mirror leaves inside one composite parameter.
  Raw/stale/foreign identities, absent branches, and malformed paths reject;
  duplicate immutable references remain legal data and grant no prepare/start
  authority. Every edit is
  transactional; any graph edit detaches the graph-bound reference trace.
  Canonical replay-backed undo/redo retains bounded complete `ALGS` session
  snapshots across graph, probe, trigger, sidecar-import, cached-job,
  component, hierarchy, and source-map state. Every navigation target reruns
  complete core replay plus UI semantic/catalog admission. Per-probe
  canonical name, retained-sample, and decimation-stride editors preserve
  stable source/type identity and reject any capture policy that cannot hold
  its active trigger window. A shared aggregate-bounded replay projection now
  applies those policies to the plotted exact samples and carries exact
  root-clock time across rate domains. The horizontal axis and cursor retain
  that exact shared time across mixed local clocks; pointer coordinates only
  select an existing exact time, and per-series values use explicit
  sample-and-hold. Nonempty Boolean and physical-scalar probes—exact rationals,
  measured intervals, and signed/unsigned canonical lattice counts—including
  external-source outputs are plotted by canonical name without a fixed-series
  whitelist. Physical scalars with the same registered type share one scale;
  distinct types receive deterministic separate panes with their canonical type
  name and unit while retaining the shared exact-time axis and cursor. Browser
  local storage atomically preserves one canonical
  [`ALGS` V1 authoring session](docs/GRAPH-AUTHORING-SESSION-V1.md): the control
  `ALGW`, bound `ALGP`, catalog-bound composite cached-job `ALGW`, selected
  `ALGC`, complete `ALGH`, and freshly replayed `ALGM`. The `algs1:` wrapper has
  one lowercase-hex payload; the retired `algwb1:` format is unsupported.
  Native/browser `.algs`, `.algw`, `.algp`, `.algc`, and `.algm` exchange
  imports only after the artifact's bounded full replay plus every applicable
  identity, catalog, hierarchy-context, and audited UI admission check.
  A separate canonical `ALGC` V1 authoring package now embeds that unchanged
  workspace, validates typed public connector mappings, and binds a bounded
  integer front panel to exact parameters and public outputs. The visible
  reference component supplies eight exact PID/interlock controls and seven
  exact replay indicators (four rational and three Boolean); invalidating a
  binding detaches the panel without weakening or rejecting the underlying
  workspace draft. A canonical panel editor now adds any unowned exact
  input/parameter/output binding with a fresh monotonic identity, edits stable
  name/binding/integer rectangle metadata, removes items without rewinding the
  cursor, and drags headers with one cumulative exact-coordinate commit. Every
  accepted change replaces `ALGC` through complete `ALGH`/`ALGM`/`ALGS`
  admission and unified history while retaining the embedded workspace,
  connector pane, root hierarchy, probes, and cached-job workspace.
  Canonical `ALGH` V2 binds scoped root/component placeholders to exact
  component digests, rejects recursive definition cycles, bounds depth and
  expanded occurrences, and deterministically flattens the visible two-level
  wrapper/PID hierarchy to an ordinary audited 21-node/25-wire workspace with
  fresh monotonic identities. Canonical `ALGM` V1 then maps every final node
  and wire back to one exact root or component-occurrence origin. Its import
  path freshly flattens the complete `ALGH` and regenerates every byte before
  the selected-node inspector or exact trace cursor displays source-path/final
  endpoint correlation; it grants no execution or firmware authority. A
  visible flattened-source browser enumerates every final node and wire from
  that fresh map. Opening one rechecks its exact final-to-origin mapping and
  resolves every component occurrence path through the current `ALGH` before
  scrolling to the root canvas, selected library definition, or authoritative
  control canvas. Root items are highlighted directly; component nodes select
  their local node, while component wires select and distinctly highlight the
  exact local wire and retain its target node in the existing inspector. These
  focus, scroll, and selection facts are transient: canonical `ALGS`, history,
  and persistence remain byte-identical, and stale provenance rejects without
  retaining a false destination.
  The component-library panel lists exact dependencies already admitted by
  that `ALGH`, exchanges the selected dependency as canonical `.algc`, imports
  a bounded standalone leaf only after exact replay and audited graph
  admission, and constructs a named version-1 empty `ALGC` directly from the
  current exact schema/clocks with all monotonic cursors at one. A conflicting
  stable name rejects; recreating byte-identical empty content is a
  selection-only no-op. The panel removes only an unreferenced
  non-authoritative dependency and can add or delete root occurrences directly.
  The same panel edits the selected dependency's stable name and declared
  behavior version as one canonical metadata transaction. Versions are
  canonical nonzero decimal `u32` values and may stay unchanged or increase,
  never regress; a name already owned by another dependency rejects, and an
  identical name/version pair is an exact no-op. Accepted edits recursively
  replace every affected parent and binding by digest, regenerate `ALGH` and
  `ALGM`, and reconcile transient selections and source focus without aliases.
  Each accepted action edits a cloned
  hierarchy, retains monotonic identities, regenerates and admits the complete
  `ALGH`/flattened `ALGW`/`ALGM` branch, and records the prior complete `ALGS`
  before committing. A structural root canvas renders the canonical root
  `ALGW` without assigning behavior to placeholders. Bound component headers
  drag onto the exact integer presentation lattice; selecting an output and a
  type-compatible input creates one monotonic root wire, while secondary-click
  disconnects an owned input without rewinding the wire cursor. Placement and
  wiring candidates pass the same complete flatten/source-map/semantic/session
  transaction, and exact duplicate imports or placements are selection-only
  no-ops.
  A separate selected-definition canvas follows the exact non-authoritative
  dependency chosen in that library panel. It reuses the audited palette,
  stable node and wire allocators, schema-directed metadata editors, and exact
  integer dragging against the dependency's embedded `ALGW`. An existing
  library dependency can be added as a child through one transaction that
  creates its fresh monotonic placeholder and scoped `ALGH` binding together.
  The same selector can rebind one selected child placeholder to another exact
  library dependency. Stable child connector IDs preserve every compatible
  parent-local wire and public endpoint while the placeholder node, label,
  placement, and allocation cursors remain exact. A same-shape replacement
  changes only the scoped binding at the parent-definition boundary; a changed
  public shape recursively replaces affected parent identities. Both paths
  freshly flatten and regenerate `ALGM` before one complete-session commit.
  Missing or incompatible live connectors, cycles, limits, and indirect
  control-authority replacement reject atomically, while selecting the already
  bound child is an exact no-op.
  `ALGH`-owned child placeholders remain visible and wireable; the dedicated
  inspector action removes a placeholder, its incident wires, and its scoped
  binding together, while an ordinary node deletion remains invalid and live
  public connector or panel bindings veto removal. Accepted edits replace the
  selected `ALGC`, retain logical selection through every recursive digest
  remap, and commit regenerated `ALGH`/`ALGM` in complete `ALGS` history while
  preserving control, probe, cached-job, and unchanged-root authority exactly.
  Compatible control edits replace the selected `ALGC` dependency and remap
  every exact binding while preserving the authored root workspace and stable
  library entries; invalid imports, removals, replacements, or selections
  leave the prior session unchanged.
  Selected dependencies now also export and import canonical `ALCP` V1
  packages. Each package contains exactly one selected root component, its
  transitive `ALGC` closure, and the component-scoped bindings that standalone
  `ALGC` cannot represent. Import never replaces an existing identity or
  binding: exact duplicates are selection-only no-ops, while stable-name or
  parent/node child conflicts reject atomically. A changing merge advances
  `ALGH` once, regenerates flattened `ALGW`/`ALGM`, passes complete library and
  session admission, and enters unified undo/redo/persistence as one edit.
  A separate capability-derived target palette now intersects authenticated
  firmware opcode/resource facts with the reviewed deployment registry. The
  visible TinyBee reference admits only GPIO22/32/33/35 stable Boolean reads
  into a separate Realtime draft. Its executable paired-input node initially
  binds GPIO22 permit and GPIO32 interlock fields and feeds a required sink.
  Every bounded schema-aware rebind to an unused exact catalog entry preserves
  its sibling and stable node/placement identity, reruns complete semantic and
  capability admission, and independently decodes a fresh fixed 4 KiB firmware
  package before committing either ALGW or ALGR identity.
  Current membership in the exact catalog is a prerequisite, so raw handles,
  labels, and numeric GPIO text grant no authority. No broader pin/peripheral
  inventory is inferred. A distinct
  board-name-independent explorer now decodes the complete
  bounded capability ledger into 62 TinyBee resources, 51 aliases, ownership,
  safe/hazard facts and supporting-section counts while retaining that four-item
  graph access set as a visibly narrower authority. Search and filters separate
  graph-readable, graph-closed, hazardous, Service and Realtime resources. The
  physical package has no licensed visual, so the UI explicitly draws no board
  shape or hotspot and keeps physical placement/HIL authority closed. The
  host-only simulator separately publishes a small CC0 diagnostic PNG and four
  GPIO hotspots to exercise digest-bound acquisition, dimension-checked
  rendering, picking, and live typed-resource linkage without resembling or
  claiming a PCB photograph. Canonical `ALGP` V2 sidecars bind bounded
  diagnostic probes to exact workspace outputs and retain one replay-only
  Boolean edge trigger with a bounded pre/post sample window. Probe/trigger
  edits filter host plots without mutating the graph or granting firmware
  telemetry/resource access or device-trigger authority. Physical-scalar
  series use certified analog enclosures on independently scaled type panes;
  measured intervals retain both exact endpoints, and canonical counts are
  multiplied by their registered exact quantum while retaining the count in the
  cursor label. Boolean series use aligned high/low logic-analyzer lanes. All
  literal non-scalars use categorical state/event lanes with a marker for every
  retained event and a stronger marker only for byte-exact canonical value
  changes. Bounded previews, full typed-value SHA-256 identities, canonical
  byte counts, and original local clock/tick/sequence remain visible without
  assigning those values an analog ordering. Same-time events are counted on
  one exact-time marker instead of being given invented sub-tick positions. All
  panes share the trigger-selected exact-time window, trigger marker, and
  cursor. Plot coordinates come only
  from certified `f64` enclosures, and cursor labels retain exact sample values.
  Stored or imported sidecars for
  another workspace fail closed without partial graph/probe mutation; exact
  no-op sidecar edits do not trigger redundant persistence.
  This remains editor state, not deployment or firmware authority. See
  [`docs/GRAPH-WORKSPACE-V1.md`](docs/GRAPH-WORKSPACE-V1.md).
  The component/front-panel boundary is in
  [`docs/GRAPH-COMPONENT-V1.md`](docs/GRAPH-COMPONENT-V1.md).
  The component-instance/flattening boundary is in
  [`docs/GRAPH-HIERARCHY-V2.md`](docs/GRAPH-HIERARCHY-V2.md).
  The reusable nested component exchange boundary is in
  [`docs/GRAPH-COMPONENT-PACKAGE-V1.md`](docs/GRAPH-COMPONENT-PACKAGE-V1.md).
  The canonical total hierarchy source-map boundary is in
  [`docs/GRAPH-HIERARCHY-SOURCE-MAP-V1.md`](docs/GRAPH-HIERARCHY-SOURCE-MAP-V1.md).
  The authenticated resource-palette boundary is in
  [`docs/GRAPH-CAPABILITY-CATALOG-V1.md`](docs/GRAPH-CAPABILITY-CATALOG-V1.md).
  The descriptive-versus-operational board boundary is in
  [`docs/BOARD-EXPLORER-V1.md`](docs/BOARD-EXPLORER-V1.md).
  The diagnostic-probe sidecar is in
  [`docs/GRAPH-PROBE-V2.md`](docs/GRAPH-PROBE-V2.md).
- Canonical `ALGT` V1 traces bind the graph digest, semantic/implementation
  registry digest, and inclusive root-clock horizon. Replay decodes only the
  external authority, reruns the fixed simulator, and requires every regenerated
  byte to match; it grants no firmware or deployment authority.
- A separate deployment registry lowers one fixed Boolean Stream subset into
  the sibling firmware's canonical 4 KiB `ALGRIR02` package. Production limits
  are derived from the complete authenticated target capability document, not
  guessed defaults. The compiler binds its identity, exact split arenas,
  opcode/resource palettes, audited semantics, fixed implementations/WCET,
  graph, target MCU, and configuration; proves integer device-cycle periods,
  executor reserve, topology, and exact arena use; then requires the
  allocation-free firmware decoder to replay the package.
- A native cross-repository fixture sends those exact compiler bytes directly
  into the sibling portable firmware runtime. Exact identity/capacity admission,
  safety-gated Service tick-zero priming, unique Service/Realtime owners, and
  1 kHz→500 Hz queue/latest/sink execution reproduce the expected Boolean
  samples without a graph-document interpreter or physical side effect.
- A second cross-repository fixture lowers a typed TinyBee GPIO33 resource
  handle into the first physical opcode and runs it through the firmware actor
  types. GPIO34 and a mismatched target capability digest fail before package
  authority; runtime admission rechecks the same exact opcode/class/access/
  selector palette. This is host functional evidence, not physical input HIL.
- A paired-resource fixture lowers ordered TinyBee GPIO22/GPIO35 handles into
  one Realtime conjunction opcode and required sink. The permanent firmware
  actor types read both resources in order on every release—even when the first
  is false—and emit only after both are present. Duplicate or unadvertised
  selectors reject before execution. Canonical bounded `ALGRREP1` artifacts
  retain the exact input frames, actual provider-call order, completion/fault
  reports, run identity, target tuple, implementation, and package. Imports
  rebuild fresh firmware actors and must reproduce every byte; the TinyBee UI
  exports both its four-case success transcript and unavailable-input fault
  transcript without acquiring device authority. See
  [`docs/GRAPH-DEPLOYMENT-REPLAY-V1.md`](docs/GRAPH-DEPLOYMENT-REPLAY-V1.md).
- The headless and WASM clients publish that fixed package, reconcile independent
  dual-core installation, and drive exact future start/stop epochs. Running is
  reported only after both permanent actors and the shared bridge agree; the
  first execution fault is retained while an exact stop is reconciled.

The selected local revisions and any uncommitted source state are recorded in
[`docs/HYPER-BASELINE.md`](docs/HYPER-BASELINE.md). A dirty local source tree is
valid for development but cannot qualify a reproducible compiler release.
The current curve and metric contract is in
[`docs/EXACT-TOOLPATH.md`](docs/EXACT-TOOLPATH.md).
The configuration-derived scheduling and executor-preflight contract is in
[`docs/EXACT-MACHINE-SCHEDULING.md`](docs/EXACT-MACHINE-SCHEDULING.md).
The visible offline machine/CAM and transactional artifact boundary is in
[`docs/OFFLINE-MACHINE-CAM.md`](docs/OFFLINE-MACHINE-CAM.md).
The selected exact UI-only CNC source boundary is in
[`docs/EXACT-CNC-GEOMETRY-IMPORT.md`](docs/EXACT-CNC-GEOMETRY-IMPORT.md).
The immutable block/cache boundary is in
[`docs/CACHED-PARTITIONS.md`](docs/CACHED-PARTITIONS.md).
The global participant/manifest boundary is in
[`docs/GLOBAL-JOB-MANIFEST.md`](docs/GLOBAL-JOB-MANIFEST.md).
The authenticated browser/cache boundary is in
[`docs/WIFI-CACHE-DELIVERY.md`](docs/WIFI-CACHE-DELIVERY.md).
The dedicated control-worker boundary is in
[`docs/LIVE-CONTROL-WORKER.md`](docs/LIVE-CONTROL-WORKER.md).
The first typed graph and canonical replay boundary is in
[`docs/TYPED-GRAPH-V1.md`](docs/TYPED-GRAPH-V1.md).
The exact-CAM development evidence is in
[`docs/CHECKPOINT-EXACT-CAM.md`](docs/CHECKPOINT-EXACT-CAM.md).

The first offline board-diagnostic view now independently decodes canonical
bounded resource overview and digital edge-capture records, reconciles them to
the complete TinyBee capability, and cross-links ledger selection with an exact
integer-cycle trigger plot. Its current fixture is prominently simulation-only
and grants no board connection, measurement, lease, command, or output
authority. See the [offline diagnostic explorer
checkpoint](docs/OFFLINE-DIAGNOSTIC-EXPLORER.md).

The same canonical diagnostic records now have a typed authenticated client
lifecycle. Subscription state exposes monotonic event/loss progress; capture
state reconciles ambiguous configure/arm/stop responses and reconstructs a
retained record from exact digest-bound ranges before exposing it. In-memory,
signed HTTP-fixture, and real localhost TCP/HTTP tests pass without contacting
the board or WLAN. Immutable capability acquisition now supplies the visible
worker/UI with target context. Canonical authenticated polling connects the
telemetry machine to low-rate live status and sampled logic lanes, while the
capture machine drives exact retained traces; a future WebSocket can reuse the
event contract but is not a prerequisite. See the
[authenticated diagnostic client checkpoint](docs/AUTHENTICATED-DIAGNOSTIC-CLIENT.md).

The headless client now also owns the firmware's passive runtime-health
boundary. It requests the exact zero-configuration `HealthSnapshot`, validates
the fixed queue and dual-executor stack reports again, retains monotonic
boot-scoped evidence across temporary real-time-report absence, and exposes
integer queue/headroom facts without converting them to percentages or safety
authority. The dedicated worker now polls no faster than its explicit 1–60
second policy after successful heartbeats, resets boot/session-scoped evidence,
keeps health failures separate from clock qualification, and renders the last
valid queue and executor-stack facts in the live-device panel. Its strict JSON
projection is validated again in the rendering realm before insertion.

The worker now also requests the selected MCU's canonical `ALMCAP04` document
through authenticated `CapabilitiesGet` ranges. It repeats the exact range
after ambiguous loss, freezes the digest after discovery, bounds allocation,
and independently decodes and hashes the complete document. Schema v3
introduced one transfer per worker generation; current schema v5 adds telemetry
and capture documents while preserving that capability contract. The rendering
realm validates them again before constructing the board-name-independent
explorer. The live
panel shows exact board, revision, chip, core, memory, resource, hazard, visual,
hotspot, HIL, and armability facts. It separately exposes passive diagnostic
observations—including fixed budgets, cadence, and freshness—and graph
operations while granting no resource lease, raw acquisition, output, arm
transition, or safety authority.

Capability V4 adds a third independent digital-capture catalog. The worker now
derives selected channels, exact acquisition sources, trigger support,
transition capacity, configure/record/chunk bytes, duration, and arm horizon
from that authenticated catalog. The browser revalidates retained sources and
budgets before rendering. The host fixture identifies as
`sim-mks-tinybee-v1`; the physical TinyBee package remains capture-absent.

## Value domains

The core intentionally keeps four domains structurally separate:

1. exact `hyperreal::Real` CAD/CAM values with compile-time units;
2. bounded measured values expressed as exact rational closed intervals;
3. canonical firmware values expressed as integer counts/ticks and
   `alumina-machine-ir` records; and
4. finite lossy display values produced only by named projection functions.

There is no conversion from a renderer value into an exact or canonical value.
A compile-fail documentation test enforces that boundary. The complete policy
is in [`docs/VALUE-BOUNDARIES.md`](docs/VALUE-BOUNDARIES.md).

## Build and test

The normal verification path is offline once dependencies are present:

```sh
cargo test --workspace --offline
cargo clippy --workspace --all-targets --no-deps --offline -- -D warnings
cargo check --workspace --target wasm32-unknown-unknown --offline
scripts/audit-source-policy.sh
env -u NO_COLOR trunk build --release --locked --offline
```

Run the desktop shell with:

```sh
cargo run --offline
```

Run a browser development server with:

```sh
trunk serve --release --offline
```

The production bundle is written to ignored `dist/` and includes compressed
assets suitable for later embedding in `alumina-firmware`.

## Scope after this checkpoint

The next interface milestones add native/tighter broader-curve metric carriers,
certified nonzero-radius blends, direction-aware and broader-axis kinematics,
complete public device/security/machine-membership discovery,
physical-browser/radio qualification, crash-durable authoring journals and
nonterminal owner recovery,
annotated board photography, higher-rate/triggered oscilloscope and
logic-analyzer acquisition, analog telemetry, groups, child-occurrence
rebinding,
connector-shape authoring/remapping, front-panel runtime injection/execution,
responsive/grouped panel layout, executable composite
physical-resource consumers, prepare/start job nodes, conflict-aware shared
workspace persistence,
broader deterministic host graph behaviors, and fixed-memory authenticated
Service/Realtime upload/core transfer and task
composition, additional resource opcodes and capability-generated graph nodes,
and physical input/timing qualification. Raw G-code remains an optional exact
UI importer, never firmware or canonical job input.

This repository is MIT licensed. Dependencies are restricted to permissive
licenses accepted by the Alumina project; GPL-family code is excluded.
