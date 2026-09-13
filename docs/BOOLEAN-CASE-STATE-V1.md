# Exact Boolean case and visible state V1

This slice adds the smallest audited `HostExact` subset needed to express a
conditional state machine without hiding state or priority inside an opaque
node. It does not add a firmware opcode, deployment authority, physical output,
or compatibility layer.

This document records that checkpoint. The subsequent
[`TYPED-CASE-STATE-V1`](TYPED-CASE-STATE-V1.md) slice generalizes constant and
case behavior and adds exact-rational bindings; the identities below remain the
historical identities of the Boolean-only checkpoint.

## Fixed semantics

Three palette kinds are admitted by exact kind and version:

- `control.bool.constant` V1 emits its exact Boolean parameter on every tick of
  its declared output Stream clock;
- `control.bool.case` V1 requires `selector`, `when_false`, and `when_true`
  Boolean Streams on one identical clock and emits exactly one selected Boolean
  value; and
- `control.bool.delay` V1 uses the existing typed `UnitDelay` behavior with an
  exact Boolean initial parameter and a matching `NodeStateContract`.

The case has three required bounded Stream queues and declares feedthrough from
all three inputs. A sample must exist on both branches at every evaluated tick.
Missing data, wrong types or clocks, duplicate port identities, incomplete
dependencies, wrong parameters, undeclared state, and insufficient state
storage all reject before execution.

All delays emit prior state before combinational evaluation and capture next
state only after the current tick settles. The implementation now admits a
delay for any identical typed input/output Stream whose parameter and declared
state type match the Stream sample type; the exact-rational delay behavior is
unchanged.

## Reset-dominant composition

The regression graph builds a latch only from visible nodes:

```text
set_case = case(set, current, set)
next     = case(reset, set_case, false_constant)
current  = delay(next, initial=false)
```

`case(selector, when_false, when_true)` makes reset priority explicit: when
`reset` is true, `next` is false even if `set` is simultaneously true. The
tested current-state sequence is:

```text
false, false, true, true, false, false, true
```

The conflict occurs at the fourth tick: current state is still visibly true
for that read-before-write tick and next state becomes false. A later set can
arm it again. Reversing all caller samples produces the identical simulation,
and canonical `ALGT` replay independently regenerates the trace.

## UI and identity

The audited editor palette grows from 13 to 16 entries. Boolean constant and
delay prototypes default to false; the case has no hidden parameter. Palette
creation, Boolean parameter editing, malformed literal rejection, monotonic
node identities, and transactional draft behavior are covered by UI tests.

The representative PID/interlock graph itself is unchanged, so its `ALGR`
identity remains
`96a3348264a9b65d267b45f9a6419a44ee60473fd961abcf4436295e10b3735f`.
Adding the three audited kind bindings changes the `ALSI` identity to
`5bef8dfb024b548cecd6fb023aba789aeca1a290809fe4bf022559cb6b3d3b16`.
The resulting 8,292-byte reference `ALGT` identity is
`80c98efb242980eab77ae750b7bfa3736cf0bf69496c0053233c17c5ff2da283`.

## Deliberate boundary

This is a Boolean dataflow case, not a general LabVIEW case structure. Typed
case tunnels for other values, structured loops, richer state records, event
semantics, state-machine authoring conveniences, fixed-memory lowering, WCET
proof, and separately reviewed Service/Realtime firmware implementations remain
later work.
