# Exact UI-only CNC geometry import

Snapshot: 2026-08-13.

The browser/WASM Machine/CAM workspace can now import one selected, connected
XY line/arc program as exact Hypercurve geometry. This is an optional source
adapter—not a firmware protocol, a GRBL/FluidNC compatibility surface, or an
alternate job format. Firmware continues to receive only canonical integer
machine IR and immutable cached partitions.

## Exact source boundary

`import_exact_cnc_geometry` accepts bytes under a caller-owned
`CncGeometryImportLimits`. It rejects non-ASCII/control bytes, overlong source
or physical lines, excess lines/words/curves, overwide decimal tokens, malformed
or nested comments, and inconsistent `%` envelopes before unbounded geometry
growth. Temporary line/word/provenance/curve storage uses checked counters and
fallible reservations.

Every coordinate token is parsed directly into `hyperreal::Rational`; no
`f32`, `f64`, renderer coordinate, locale conversion, or tolerance participates.
Millimetres are identity-scaled and inches use the exact `127/5 mm` conversion.
The importer then constructs native `LineSeg2` or `CircularArc2` objects and one
connected `CurvePath2`. Hypercurve rejects zero-length lines, zero-radius arcs,
radius mismatch, and invalid path topology at the exact source boundary.

## Deliberately selected semantics

Accepted words are:

- `N` with a non-negative integral `u32` block number;
- `G0`, only to establish or refine the pre-cut position before retained motion;
- connected `G1`, `G2`, and `G3` XY geometry;
- explicit `G17`, `G20`/`G21`, `G90`/`G91`, and for arcs
  `G90.1`/`G91.1` modal state;
- exact `X`, `Y`, `I`, and `J` decimal coordinates; and
- one terminal `M2` or `M30`, optionally inside a paired `%` envelope.

The first position must be an absolute `G0` that supplies both X and Y. Arcs
require both I and J. `R` arcs and start-equals-end full circles remain outside
V1 because their ambiguity or sweep choice is not silently guessed.

Every other address/code fails closed. That includes feed, Z/other axes,
spindle, tool, compensation, canned cycles, process state, checksums, macro
expressions, and a rapid after retained geometry. The importer therefore does
not claim to execute a CNC dialect or preserve machine/process behavior.

## Source provenance is not canonical identity

The import report retains:

- SHA-256 of the exact raw source bytes;
- admitted byte, line, word, positioning-block, and curve counts;
- exact retained start/end points; and
- for every curve, source line, optional N number, motion family, units,
  endpoint mode, and I/J mode.

Those facts support UI review but do not enter firmware. `ALMEVD01` separately
binds a canonical exact-rational encoding of the resulting line/arc geometry.
Tests compile the direct Hypercurve fixture, the equivalent CNC text, and a
comment-only variant: all three produce the same exact-source digest, cached
partition, and evidence identity, while the two text sources have different
raw-source hashes. This proves that comments, formatting, and legacy text are
not machine authority.

## Transactional Machine/CAM admission

Native/browser file selection and the bounded editor move untrusted `.nc` bytes
only. An imported candidate becomes visible only after this complete chain:

```text
bounded raw source
    -> exact selected-semantics parser
    -> native Hypercurve connected path
    -> current ALMCFG05-derived machine profile and error budget
    -> exact native-extrema travel proof
    -> Hyperpath/Hypersolve lookahead and jerk replay
    -> bounded step/tick lowering and post-rounding travel proof
    -> production StepperExecutor preflight
    -> canonical cached partition and independent event replay
    -> reconstructed ALMEVD01 evidence replay
```

Any failure leaves the existing source, schedule, cache, replay, evidence, and
selection untouched. Regression tests reject an `F` process word before CAM and
reject a syntactically valid 301 mm line at the machine-travel proof, retaining
the prior workspace in both cases.

## Verification

The checkpoint passed:

- 28 application tests, including the direct-fixture/text/comment identity
  comparison, transactional process/travel rejection, and a complete headless
  egui frame;
- 106 exact-core tests, including five importer tests for exact line/arc
  construction, inch/incremental conversion, selected-semantics rejection,
  admission limits, and every strict prefix of the representative program;
- 37 protocol-client tests, the exact-control integration test, and the
  compile-fail display-to-exact boundary test;
- strict native and `wasm32-unknown-unknown` Clippy, strict rustdoc, complete
  WASM test-target linking, and the local-source/permissive-license audit;
- an optimized 5,345,732-byte WASM validated by `wasm-tools`, with
  2,409,207-byte gzip and 1,931,517-byte Brotli forms that both decompress to
  SHA-256 `0ffaa504dd5cb0970b12788916a9135517252092ea8fd0474d0fc41243b34c5d`;
  and
- a loopback-only software-WebGL Chromium render showing the source authority,
  selected-semantics/limit panel, bounded file/editor controls, exact geometry
  identity, path, machine schedule, cache replay, and non-armable safety state.

No WLAN association, board connection, serial/USB operation, reset, flash,
output, or physical qualification occurred.

## Current limits

- One connected two-axis line/explicit-IJ-arc path is imported.
- The interactive policy permits 1 MiB source, 4 KiB per physical line, 65,536
  lines, 262,144 words, 4,096 curves, and 128 characters per decimal.
- At most 128 provenance rows render at once; all admitted rows remain retained.
- General Bezier/NURBS source import, multi-contour job organization, tool and
  work transforms, process semantics, and generated G-code export from exact
  geometry remain future UI work; the editor can only download its unchanged
  draft bytes.
- Raw source provenance is not yet embedded in canonical schedule evidence;
  exact resulting geometry is.
- This is native/WASM software evidence only. It does not contact, arm, reset,
  flash, or qualify a board.
