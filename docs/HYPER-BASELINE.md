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
| `hypercurve` | `a7cb2f91a79406a0a98fcb58815fc2e4e3e3da41` | `src/bezier_offset.rs` modified by concurrent local development |
| `hyperpath` | `e65506279d3cba99a23cf98bbd17be44126ec14d` | clean |
| `hyperphysics` | `a8002f286914356d3ebc5f491695f39f6f1c029e` | tracked source and tests modified by concurrent local development |
| `hypersolve` | `cdac9bf4e5b88aa050d53667bc2c2244db5ee650` | clean |
| `hypergraphics` | `b7f197dfe8ddac5112a4411db33815598905f973` | clean; includes checked native Hypermesh adapter |

## Qualification rule

The working trees, rather than a published CSGRS release, are the development
authority. A release/job compiler identity must include a coherently reviewed
revision set plus source-tree digests. Any tracked modification, including the
current Hypercurve and Hyperphysics work, makes this table a development
snapshot rather than a reproducible release pin. Untracked fuzz corpora and
build executables are
excluded from Cargo package sources but must still be removed or explicitly
excluded before a whole-tree release digest is generated.

The baseline is advanced only as one set: update paths/patches if needed, run
native and WASM compiler fixtures, run the full license scan, update every row,
and then qualify the new set. Do not substitute a crates.io CSGRS version when a
local checkout is temporarily between compiling states.

## Verified dependency boundary

The root manifest directly selects local CSGRS, Hypergraphics, Hyperpath,
Hyperreal, and Hypersolve where they have a concrete I0 role. CSGRS and those
crates bring the mutually compatible local Hypercurve, Hyperlattice, Hyperlimit,
Hypermesh, and Hypertri packages transitively. The exact core also uses the
local Hyperphysics package transitively. The exact core also uses the local
`alumina-machine-ir` and protocol crates, so canonical firmware values are the
firmware's real integer/tick types rather than a UI duplicate.
