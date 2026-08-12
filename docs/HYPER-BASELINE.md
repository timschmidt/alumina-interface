# Local CSGRS/Hyper baseline

Snapshot: 2026-08-12, from the shared workspace at
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
| `hypercurve` | `dc7aff02fd483fb532765c7e539cfeeddba7d57b` | tracked source modified by concurrent local development; tracked diff SHA-256 `3c5765f7c7c7d07935a3aa0a86e95e66597828c6846efc947758e29cdd6e1d9e` |
| `hyperpath` | `e65506279d3cba99a23cf98bbd17be44126ec14d` | clean |
| `hyperphysics` | `a8002f286914356d3ebc5f491695f39f6f1c029e` | tracked source and tests modified by concurrent local development; tracked diff SHA-256 `99766a9ad8ccb54b8eac523fcc904db4d2df3aa5eeb4c10f5bcb781d57ad9667` |
| `hypersolve` | `d8bfa6b113020d1588ce2b0e549235d1bb9bc205` | clean |
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

The recorded Hypercurve and Hyperphysics tracked-diff digests were identical
immediately before and after this checkpoint's native, WASM, documentation, and
optimized-bundle qualification. That establishes the tested development
snapshot; it does not convert either dirty tree into a release pin.

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
subdivision. Hypergraphics retains that certificate for one-way presentation.
The interface promotes only losslessly supported line and explicit-arc source
objects into Hyperpath metric carriers; it does not promote a renderer mesh.
