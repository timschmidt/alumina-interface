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
