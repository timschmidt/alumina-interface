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
| `hypercurve` | `de9628dd962a8dcbbe20a527f743a1d2abcff225` | concurrent tracked edits in `src/bezier_offset.rs`, `src/bezier_region.rs`, and `src/curve_region_boolean.rs`; final V3-qualified diff SHA-256 `14da56a6db9295aba186b9cb585d662241571971f7be08fde90811e5067ba71c` |
| `hyperpath` | `d792aa8dc843218b26fc0d1730033e5cd06bdf2f` | clean; adds exact N-axis affine velocity/acceleration/jerk projection and bottleneck replay to diagonal length, acceleration lookahead, monotonic transitions, and component-local jerk refinement |
| `hyperphysics` | `a8002f286914356d3ebc5f491695f39f6f1c029e` | tracked source and tests modified by concurrent local development; tracked diff SHA-256 `99766a9ad8ccb54b8eac523fcc904db4d2df3aa5eeb4c10f5bcb781d57ad9667` |
| `hypersolve` | `6ce08b714cdba1e3668e1af6c83f0a249bda9bb5` | clean |
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
artifact snapshot remains recorded in its milestone evidence. The subsequent
monotonic-jerk checkpoint observed HEAD
`08fb7fef66720b123d32cf94d3e0528eea1c83fd` with tracked diff SHA-256
`3d390387e0c88c85efdad6ce52b8c45fffac1230c33b1c70950d2283ca8542d3`.
The current jerk-feasible G1 checkpoint's final native/WASM tests, strict
checks, optimized bundle, decompression checks, loopback render, and final core
check completed against the earlier same-HEAD tracked diff SHA-256
`491ddc3ad6cd04e92a4a22ca4c21b7e1617193522e947b1169d1ecbcb96303ce`.
The subsequent affine axis-projection checkpoint's native/WASM tests, strict
checks, optimized bundle, decompression checks, corrected software-WebGL
loopback render, and final core check completed against its separately recorded
HEAD `d85eec9aa6bcde54ebbfd5ac08a3ac72d2f244e9` and tracked diff SHA-256
`c1583f1c371c28ab32f30435f4bdcd07d74e42bcd2c0d6f60bb3c1f991e4b08e`.
The exact timer-lattice checkpoint's final native/WASM tests, strict Clippy and
rustdoc, complete WASM test-target link, source/license audit, optimized bundle,
compression checks, long-budget software-WebGL loopback render, and final core
check completed against its previously recorded `72bc0c7...` Hypercurve state.
Pre/post snapshots around that gate batch and production rebuild retained the
same HEAD, tracked file set, and diff digest.

The canonical planner-evidence V3 work deliberately straddled later concurrent
sibling edits without treating them as Alumina changes. Its first complete gate
batch and optimized artifact were produced while Hypercurve was stable at
`f6508292039a0249ee63fbd0d6855ddb9ff9a0d1` with tracked edits in
`src/bezier_offset.rs`, `src/bezier_parameter.rs`, `src/bezier_region.rs`, and
`src/curve.rs`, whose binary diff SHA-256 was
`549005666bbf29a48445b583ebc97c5dfbdc8000be0e8d16fa9518d6619abcdd`.
Hypersolve was then still the clean `cec630b...` state. While loopback approval
was pending, both repositories advanced independently; intermediate core checks
continued to pass but were not used to relabel that first artifact.

After the final 64 MiB subtranscript bound was added, the complete native test
suite, native and WASM strict Clippy, strict rustdoc, complete WASM test-target
link, source/license audit, optimized build, compression integrity checks, and
long-budget software-WebGL loopback render were repeated. Pre/post observations
remained stable at the current table's Hypercurve `de9628d...` with diff
`14da56...` and clean Hypersolve `6ce08b7...`. The final V3 artifact and current
source qualification therefore share that explicit coherent dependency
snapshot. Later Hypercurve edits are expected and do not retroactively rename
or invalidate this recorded development artifact.

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
It also owns the exact forward/reverse squared-speed proposer, conservative
two-phase monotonic jerk transitions, stop-separated component-local exact jerk
refinement, arbitrary-dense-axis affine dynamic projection, and independent
Hypersolve replay. Alumina derives exact two-axis line derivatives and retains
the projection rows and bottlenecks for all-line routes; mixed curved routes
stay on the conservative direction-independent envelope. It supplies positive
caller ceilings only to lossless exact line-to-line joins. Exact tangent
predicates must still select G1 continuity; all curvature-bearing, approximated
cubic, corner, reversal, entry, and exit nodes remain zero.
