# Typed case and exact state V1

This slice generalizes the existing visible Boolean state-machine primitives
without adding an opaque controller node. One audited constant behavior and one
audited case behavior are now schema-generic; fixed Boolean and exact-rational
kind/version bindings select their concrete types. The change remains
`HostExact` only and grants no firmware opcode, deployment authority, physical
output, or compatibility interface.

## Fixed typed contracts

`TypedConstant` admits only a schema with no inputs or input channels, one
Stream output, one parameter whose registered type is exactly the Stream sample
type, one empty output dependency, no rate transition, and no state. At every
output-clock tick it reconstructs the parameter value through the authoritative
`GraphSchema` before publishing it.

`TypedCase` admits only a schema with:

- one required Boolean selector Stream;
- required false and true Stream queues whose complete sample type and clock
  are identical;
- one output Stream identical to both value branches;
- the selector on that same clock;
- one dependency naming all three distinct input ports; and
- no parameters, rate transitions, or hidden state.

Both branch samples must exist on every evaluated tick. Selection changes which
typed value is emitted, not which inputs the scheduler must prove available.
Missing samples, wrong clocks or sample types, aliased input identities,
incomplete dependencies, unexpected parameters/state, and malformed bindings
reject before execution.

The concrete reviewed registry kinds are:

- `control.bool.constant` V1 and `control.bool.case` V1;
- `control.exact.constant` V1 and `control.exact.case` V1; and
- `control.bool.delay` V1 and `control.exact.delay` V1, both using the existing
  schema-generic read-before-write `UnitDelay` behavior.

The editor palette exposes all six kinds. Boolean constants and delays default
to false, exact constants default to exact zero, and cases have no hidden
parameter.

## Exact reset-dominant register

The exact regression composes a loadable register from visible nodes:

```text
loaded  = case(load, current, data)
next    = case(reset, loaded, reset_constant)
current = delay(next, initial=7/3)
```

The reset constant is exactly `-5/7`. A simultaneous load of `13/7` and reset
therefore selects `-5/7` explicitly. The complete prior-state trace is:

```text
7/3, 7/3, 11/5, 11/5, -5/7, -5/7, 17/11
```

No binary float or approximation is introduced. Reversing all caller-supplied
samples produces an identical simulation, and canonical `ALGT` replay
independently regenerates the complete typed trace.

## Identity and deliberate boundary

The representative PID/interlock graph remains unchanged at `ALGR`
`96a3348264a9b65d267b45f9a6419a44ee60473fd961abcf4436295e10b3735f`.
Adding the two exact schemas and reusing the generic constant/case behaviors
changes the 18-binding `ALSI` identity to
`3ccd0aa5dbc2f745b897ca06e9a75717365cd977ab303e3c1ca48bdcf8f7c525`.
The resulting 8,292-byte representative `ALGT` identity is
`a89c8bfa87e7dced328d6cb6583a10a618431775bd3a51d2ab61a56997d4c177`.

This is a typed dataflow mux, not yet a graphical LabVIEW case structure.
Structured tunnels, nested diagram editing, event semantics, richer state
records, loop structures, general state-machine authoring conveniences,
fixed-memory lowering, WCET proof, and separately reviewed Service/Realtime
firmware implementations remain later work.
