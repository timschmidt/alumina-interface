# Shared interval arithmetic migration

`BoundedMeasurement<U>::from_nominal_uncertainty` delegates endpoint arithmetic
to Hyperinterval's `RationalInterval::from_nominal_uncertainty`. The wrapper
retains its unit type, exact endpoint storage, constant endpoint accessors,
equality and debug representation. Negative uncertainty still returns
`BoundaryError::NegativeUncertainty` with the existing message.

`BoundaryError` adds `Interval(IntervalError)` for retaining an unexpected kernel
diagnostic without reclassifying it as negative uncertainty. Its `source()`
returns the original typed interval error, including a further scalar cause
where present. Code matching this enum exhaustively must handle the new variant.
The rational nominal/radius constructor currently has only the negative-radius
failure; no new failure is introduced for previously valid measurement inputs.

Hyperinterval is a local workspace dependency with its rational feature. It
reuses the existing Hyperreal rational kernel. Measurements are still distinct
from exact design values, canonical machine values and display approximations.

The CSGRS `crates/hyperinterval/conformance/alumina-measurements` package maintains
independent endpoint expectations, cold/warm trace and allocation comparisons,
native cost/size probes and a Wasm replay consumer. Qualification and adoption
measurements are recorded in CSGRS's `INTERVAL_REFACTOR_PROGRESS.md` and its
referenced evidence archive.

Machine-profile `ExactInterval`, positive products/quotients, named error-budget
composition and full application browser/schedule qualification remain separate
parts of the migration. Their implementations and canonical encodings are not
changed by this constructor delegation.

The existing diagonal G1 profile test additionally fixes the canonical planner
transcript at 41,795 bytes with digest
`df893df83e90e6b53452e18586ffcaa2524a021631a395d4a6523be16263669c`.
This witness predates the unaccepted path-dynamics extraction and is retained
independently of that production migration. It supplements the existing exact
axis limits, bottlenecks, solver reports and terminal-step assertions; a digest
alone is not a geometry correctness proof.

Qualification against Interface `f5341f3`, firmware `2d68e82` and CSGRS's fixed
`c9f1f9ac`/current `e835617c` dependency graphs: all 201 core tests/doctests,
199 configured-release core tests, strict core Clippy and workspace formatting
pass. The final release test binary grows by 432 bytes and its complete `.text`
section by 224 bytes, within the recorded byte/percentage limits. Configured
release stripping is preserved; a symbol-preserving diagnostic changed generated
code and is excluded from acceptance measurements. This is a test-only assertion,
not acceptance of the archived production extraction or a full release/HIL claim.
The cumulative account and raw evidence are maintained in CSGRS
`docs/research/interval-size-accounts.json` and
`docs/evidence/2026-11-16-interval-path-dynamics-current-admission`.
