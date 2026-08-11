# Local CSGRS/Hyper baseline

Snapshot: 2026-08-11, from the shared workspace at
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
| `hypercurve` | `6cb75a0546e7b8e7b39838b42b2babd5246f6802` | clean |
| `hyperpath` | `e65506279d3cba99a23cf98bbd17be44126ec14d` | clean |
| `hyperphysics` | `a8002f286914356d3ebc5f491695f39f6f1c029e` | tracked source and tests modified by concurrent local development |
| `hypersolve` | `cdac9bf4e5b88aa050d53667bc2c2244db5ee650` | clean |
| `hypergraphics` | `31811aeb17bd2dc827db5669558f6251e0c2f2aa` | clean; includes checked native Hypermesh plus certified Hypercurve curve/path/region adapters |

## Qualification rule

The working trees, rather than a published CSGRS release, are the development
authority. A release/job compiler identity must include a coherently reviewed
revision set plus source-tree digests. Any tracked modification, including the
current Hyperphysics work, makes this table a development
snapshot rather than a reproducible release pin. Untracked fuzz corpora and
build executables are
excluded from Cargo package sources but must still be removed or explicitly
excluded before a whole-tree release digest is generated.

The baseline is advanced only as one set: update paths/patches if needed, run
native and WASM compiler fixtures, run the full license scan, update every row,
and then qualify the new set. Do not substitute a crates.io CSGRS version when a
local checkout is temporarily between compiling states.

## Verified dependency boundary

The root manifest directly selects local CSGRS, Hypercurve, Hypergraphics,
Hyperlimit, Hyperpath, Hyperreal, and Hypersolve where they have a concrete
interface-core role. Those crates bring the mutually compatible local
Hyperlattice, Hypermesh, Hyperphysics, and Hypertri packages transitively. The
exact core also uses the local `alumina-machine-ir` and protocol crates, so
canonical firmware values are the firmware's real integer/tick types rather
than a UI duplicate.

Hypercurve owns the exact line/arc/Bezier source path and certified chord
subdivision. Hypergraphics retains that certificate for one-way presentation.
The interface promotes only losslessly supported line and explicit-arc source
objects into Hyperpath metric carriers; it does not promote a renderer mesh.
