# Internal client Rust API migrations

## Owned network scan updates

The unpublished `alumina-interface-client` crate now carries
`NetworkUpdate::Scan(Box<NetworkScanResult>)` instead of an inline scan result.
`NetworkUpdate` retains `Clone`, `Debug`, `Eq` and `PartialEq`, but no longer
implements `Copy`. Move updates when transferring ownership or call `clone()`
explicitly when an independent copy is required. For a matched scan payload,
borrow through `as_ref()` or move the native value out with `*payload`.

The largest payload no longer determines the stack size of every status,
mutation and retry event. A successful scan update allocates one box; cloning
that variant allocates an independent box. Other update variants do not add
allocations. The client's retained latest scan remains an independent native
value, so resetting the client cannot invalidate an already returned update.

Native wire formats, scan bounds and canonical ordering, request/transaction
validation, credential redaction, worker JSON snapshots and browser behavior
are unchanged. This is an intentional internal Rust ownership/API change,
not a wire-format migration or a source-compatibility claim for external code.
The strict Clippy policy remains enabled without a lint exemption.
