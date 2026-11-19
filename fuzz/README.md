# Machine profile and lowering fuzz targets

These targets exercise the production configuration validator, exact machine
profile, scheduler, firmware V1 lowering, canonical partition/evidence construction
and cached stepper executor. They do not arm or connect to hardware. They add
qualification for the interval-refactor plan; they do not accept pending arithmetic
or dynamics migrations.

Run deterministic reachability and fresh-process controls on the supported host
compiler:

```sh
cargo +1.95.0 test --locked --manifest-path fuzz/Cargo.toml
cargo +1.95.0 clippy --locked --manifest-path fuzz/Cargo.toml --all-targets --all-features -- -D warnings
```

With cargo-fuzz and a nightly toolchain available, copy the retained seeds into a
writable corpus and run bounded campaigns from the repository root:

```sh
mkdir -p fuzz/corpus/machine_profile fuzz/corpus/machine_lowering
cp fuzz/seeds/machine_profile/* fuzz/corpus/machine_profile/
cp fuzz/seeds/machine_lowering/* fuzz/corpus/machine_lowering/
cargo +nightly fuzz run machine_profile --features fuzzing -- -max_len=64 -max_total_time=60 -timeout=10 -rss_limit_mb=8192
cargo +nightly fuzz run machine_lowering --features fuzzing -- -max_len=64 -max_total_time=60 -timeout=10 -rss_limit_mb=8192
cargo +1.95.0 run --locked --manifest-path fuzz/Cargo.toml --example replay_corpus -- machine_lowering fuzz/corpus/machine_lowering
```

Default cargo-fuzz runs include address instrumentation, debug assertions and
integer overflow checks. Keep crash/timeout inputs, then reproduce and minimize
without removing the original. A finite smoke run is not a coverage threshold or
universal correctness claim. Time/RSS limits are external containment, not internal
geometry resource bounds. Replay reports successful stages separately from typed
rejections, rather than counting every executed input as successful lowering.

## Input grammar and expectations

Inputs above 64 bytes are explicitly skipped. Missing bytes are zero.
`machine_profile` uses byte 0's high bit to select a one-byte mutation of the
original canonical wire document, with bytes 1–2 selecting an offset and byte 3
a nonzero XOR mask. Its digest is recomputed so the validator exercises syntax,
canonical form, capability and semantics beyond stale-checksum rejection.

Otherwise, the profile target and all lowering inputs use a structured grammar:
byte 1 selects an axis, byte 2 selects 1–32 microsteps, and bytes 3–4 select
uncertainty 0–7 divided by 1–4. Byte 0's bit 6 requests invalid zero microsteps.
Bit 5 moves that uncertainty to calibration scale, dividing it by another million
(parts per million), while keeping microsteps exact. This arm exercises valid
uncertain profiles. Without bit 5, nonzero microstep uncertainty is rejected because
microsteps must be exact integers. Fractions are normalized by Hyperreal before
canonical wire encoding, retaining their exact value. All other source facts remain
those of the validated fixture. The encoder can
reject invalid scalars before document parsing; that remains InvalidConfiguration.
Successful profiles must match an independently reduced exact command-density
formula and a separately decoded profile, with original configuration/capability
identity preserved. The original configuration and provenance live in fixtures/.

Byte 10 varies the selected axis's travel per output turn around its nominal
1/500 metre. Values 0–127 express a relative radius in parts per million.
When the high bit is set, the low four bits express a radius in eighths of
nominal travel. Radii of eight eighths or more must fail profile derivation.
On a positive travel interval the oracle cross-multiplies each density bound
by the opposite relative travel endpoint and checks the metre-to-millimetre
conversion. The retained one-ppm lowering seed exercises cached execution.
The coordinated consumer check runs these deterministic fixtures.

For lowering, byte 5 selects the ordinary 1/10 mm total error request or a tiny
request that must remain a typed rejection when insufficient. Bytes 6–7 choose
the endpoint of one exact line from (0,0), with each coordinate in 1–4 mm.
The source/metric limits are eight elements and depth four. Byte 8 chooses
4096 or two lowered points. Existing production timer-dilation bounds remain in
force. A completed lowering must replay through the production stepper preflight
and immutable cached partition executor. It must retain strict tick order,
contiguous segments and source identities. Canonical evidence rebuilds identically,
and altered evidence is rejected both with its old digest and a freshly recomputed
digest. Byte 9 selects the altered byte.

The corpus includes successful paths and explicit invalid-configuration, invalid
profile, insufficient-budget and point-limit cases. Unit reachability assertions
require a full cached-execution success, so early parser rejection cannot make the
lowering target appear qualified. Tests and output classify incomplete stages
separately. The grammar is a bounded qualification domain, not the full supported
configuration or curve domain; broader inputs and other plan requirements remain.
