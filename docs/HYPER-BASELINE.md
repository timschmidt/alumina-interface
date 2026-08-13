# Local CSGRS/Hyper baseline

Snapshot: 2026-08-13, from the shared workspace at
`/home/tim/Documents/GitHub/workspace`.

This interface does not resolve CSGRS or any Hyper crate from crates.io. Direct
dependencies use sibling paths and root `[patch.crates-io]` entries force the
same working trees through all transitive version requirements. `Cargo.lock`
therefore records the sibling CSGRS package (whose current manifest happens to
identify itself as 0.23.0) and current Hyper packages without registry sources.
That package label is not permission to substitute the old published release.

## Selected revisions

| Repository | HEAD | Snapshot status |
| --- | --- | --- |
| `csgrs` | `b34a2f47b90e3d329028d6337d19dfbc9629fbb0` | clean |
| `hyperreal` | `f09c147b0352884f8efe88e875c37d8f0f439ba5` | clean |
| `hyperlimit` | `b0418bddff50183fa782e5caa6da6974a2b969a1` | tracked source clean; untracked fuzz outputs and local executable present |
| `hypertri` | `86189ff6e87f056a3686d81b57952d799945663a` | clean |
| `hyperlattice` | `a475bb752c1e0fb0cfdb80f4db74a56caa6962c0` | clean |
| `hypermesh` | `088c4a4bd32bf8bfea37032432d84e19104f1ab0` | clean |
| `hypercurve` | `08fb7fef66720b123d32cf94d3e0528eea1c83fd` | concurrent tracked edits in `src/bezier_offset.rs`, `src/bezier_region.rs`, `src/curve.rs`, and `src/curve_region_boolean.rs`; post-qualification observed diff SHA-256 `3d390387e0c88c85efdad6ce52b8c45fffac1230c33b1c70950d2283ca8542d3` |
| `hyperpath` | `1e484973e25d899cc72447527fdcfeebf134d7d8` | clean; includes exact diagonal feed length, independently replayed exact two-pass lookahead, and exact monotonic nonzero-boundary jerk transitions used here |
| `hyperphysics` | `a8002f286914356d3ebc5f491695f39f6f1c029e` | tracked source and tests modified by concurrent local development; tracked diff SHA-256 `99766a9ad8ccb54b8eac523fcc904db4d2df3aa5eeb4c10f5bcb781d57ad9667` |
| `hypersolve` | `cec630b0fb121fa6ec7fe99e9780c8f020f92d61` | clean |
| `hypergraphics` | `31811aeb17bd2dc827db5669558f6251e0c2f2aa` | clean; includes checked native Hypermesh plus certified Hypercurve curve/path/region adapters |

## Qualification rule

The working trees, rather than a published CSGRS release, are the development
authority. A release/job compiler identity must include a coherently reviewed
revision set plus source-tree digests. Any tracked modification, including the
current Hypercurve and Hyperphysics work, makes this table a development
snapshot rather than a reproducible release pin. Untracked fuzz corpora and
build executables are
excluded from Cargo package sources but must still be removed or explicitly
excluded before a whole-tree release digest is generated.

The prior certified cubic-motion artifact observed Hypercurve at the same HEAD
with tracked diff SHA-256
`cd562aeada7607c31b290db51bc81025fd056cbe56d75bad443283c7328941d8`; that
artifact snapshot remains recorded in its milestone evidence. The exact
monotonic-jerk checkpoint's final native/WASM tests, strict checks, optimized
bundle, decompression checks, and loopback render completed against the same
HEAD. The table records the tracked diff observed immediately after those
gates.
Hypercurve advanced through multiple coherent and temporarily non-compiling
states while this work was underway and is expected to continue changing. This
is an observed/tested development state, not a request to hold, reset, or pin
that working tree. The Hyperphysics tracked diff remained SHA-256
`99766a9ad8ccb54b8eac523fcc904db4d2df3aa5eeb4c10f5bcb781d57ad9667`
through qualification. These facts establish development evidence; they do not
convert a moving or dirty sibling tree into a release pin.

The baseline is advanced only as one set: update paths/patches if needed, run
native and WASM compiler fixtures, run the full license scan, update every row,
and then qualify the new set. Do not substitute a crates.io CSGRS version when a
local checkout is temporarily between compiling states.

## Verified dependency boundary

The root manifest directly selects local CSGRS, Hypercurve, Hypergraphics,
Hyperlimit, Hyperpath, Hyperreal, and Hypersolve where they have a concrete
interface-core role. Those crates bring the mutually compatible local
Hyperlattice, Hypermesh, Hyperphysics, and Hypertri packages transitively. The
exact core also uses the sibling Alumina protocol, machine IR, job, storage,
clock, and runtime crates. The source audit now requires all of those packages
to resolve from the same `aluminafw` checkout, so block, cache, and `JobPrepare`
bytes cannot silently resolve to a registry or UI duplicate.

Hypercurve owns the exact line/arc/Bezier source path and certified chord
subdivision. Hypergraphics retains a separate certificate for one-way
presentation. The interface losslessly promotes lines/arcs or applies a
distinct machine-budgeted pointwise certificate to Hypercurve's exact
cubic/de Casteljau objects before constructing exact Hyperpath metric carriers;
it does not promote a renderer mesh. Hyperpath retains exact Euclidean length
for diagonal feed segments while leaving axis-specific ordering APIs strict.
It also owns the exact forward/reverse squared-speed proposer, the conservative
two-phase monotonic jerk transition for nonzero boundary feeds, and independent
Hypersolve replay. Alumina currently supplies zero caller ceilings at every
node, so only the existing four-phase rest-to-rest policy is reachable.
