# Reorganize Alumina around machine-building projects

## Summary

Deliver the GUI cleanup first, then finish the builder workflows needed for **MKS TinyBee 8 MiB bench qualification**.

Use a project tree with focused document tabs. Retain existing artifact formats and graph-session persistence. Provide an optional example library behind a non-default Cargo feature.

Success means a builder can understand their machine, prepare work, author control logic, and inspect evidence without navigating demonstration output.

## 1. Deliver the project workspace

Replace the three-view shell and permanent diagnostic sidebars with:

| Project branch | Documents and tasks |
|---|---|
| **Machine** | Connection, network setup, board and wiring, machine configuration |
| **Job** | Source/import, preview, compilation and simulation, deployment/run monitor |
| **Program** | Diagram, components and hierarchy, front panel |
| **Instruments** | Resource monitor, captures, traces and retained evidence |

- Keep the project tree on the left, document tabs in the center, and a collapsible inspector for the selected object on the right.
- Show compact connection, machine-state, qualification, and fault status persistently. Put detailed clocks, queues, digests, and certificates in contextual inspection views.
- Give each document a clear primary action and useful empty state. Explain blocked actions beside their controls.
- Split the current stacked graph screen into Diagram, Components, Front Panel, and Instruments documents. Preserve its complete-session undo/redo boundary.
- Keep document state alive when switching or closing tabs. Network sessions and polling belong to the application, independent of the visible document.
- Consolidate Open, Save/Export, and artifact inspection actions. File operations continue using their existing validators and formats.

Start with one active machine configuration, one active job source, and one graph-authoring session; expose their constituent documents and connected devices in the tree. The tree itself is transient organization, with no new project-file format.

## 2. Separate examples from normal application construction

Refactor startup in [src/lib.rs](../src/lib.rs) and workspace construction in [control_graph_ui.rs](../src/control_graph_ui.rs).

- Add an opt-in Cargo feature named `examples`. Compile the example library and shipped demonstration constructors only when enabled.
- Open examples explicitly as independent drafts. Include the geometry, CAM, PID/interlock, target-I/O, and distributed-job demonstrations.
- Default startup opens Welcome or validated restored user work. It performs no sample compilation, sample simulation, or automatic device connection.
- Extract audited registries, palette defaults, and empty-workspace construction from representative fixtures. The graph editor must work without a sample job or sample component hierarchy.
- Keep normal simulation and live diagnostic tools available in standard builds.
- Preserve regression fixtures in test support. Example-specific harnesses explicitly enable `examples`.
- Failed restore preserves the existing saved bytes and presents recovery/export actions. Imports requiring unavailable catalog bindings remain pending until matching validated artifacts are supplied.

**Release gate:** the default build supports empty workspaces and existing file workflows, with examples absent from normal navigation and startup.

## 3. Finish builder essentials after the cleanup

**Machine setup**

- Add capability-derived configuration editing for supported axes, transmissions, travel, timing, and safety-input bindings. Use exact values and existing resource-conflict validation.
- Separate local configuration drafts from authenticated active-device configuration.
- Add bounded worker commands for configuration publication/validation, commit, and rollback, using existing firmware operations. Version the strict UI/worker contract and expose transaction progress and failures.
- Reconcile ambiguous transactions through authenticated status before retrying.

**Job preparation**

- Connect the selected source and configuration to preview, compilation, simulation, and existing cached-job delivery.
- Generalize sample-only compiler entry points to accept user source and validated configuration.
- Present the current CNC importer as supported XY geometry import, with its accepted subset explained at import time.
- Put compilation failures at the relevant source or configuration field. Source/configuration changes invalidate dependent compilation and replay evidence.
- Derive deployment eligibility from current device, boot, capability, configuration, clock, and qualification evidence.

**Connection and inspection**

- Finish the visible connect → provision → inspect → reconnect workflow.
- Cross-link configuration resources, graph nodes, board resources, and trace channels by existing stable identities.
- Display simulated, measured, stale, unavailable, and faulted evidence clearly.
- Keep existing program/front-panel capabilities accessible through the new documents; broader graph execution becomes a subsequent feature milestone.

**Release gate:** complete these workflows against the deterministic simulator, including failed imports, transaction ambiguity, reboot, and connection loss.

## 4. Qualify the TinyBee at bench level

The current [board package](../../alumina-firmware/boards/mks-tinybee/board.toml) reports `compiles` and `armable = false`. Qualify the exact new release image.

- Build and embed the default-feature GUI. Record firmware/interface revisions, dependency identities, asset hashes, flash fit, and RAM/stack budgets.
- Re-establish USB communication and confirm physical revision, module identity, and 8 MiB flash before flashing.
- Run the six existing bench requirements: board identity, reset/fault/watchdog safe state, all I²S output-bit routing, XYZ limit inputs, shared-SPI SD operation, and photograph/hotspot reconciliation.
- Use disconnected-load HIL fixtures for output tests. Extend isolated fixtures where existing coverage is incomplete.
- Verify the embedded browser bootstrap, worker, AP/STA provisioning, authenticated configuration lifecycle, passive inputs, storage, and reconnect behavior.
- Retain raw captures, logs, photographs, artifact identities, analyzer reports, and reviewed run records.
- Promote qualification only after every bench requirement passes. Regenerate affected capability identities and bindings; keep physical job arming gated by later motion qualification.

**Release gate:** reviewed TinyBee bench evidence for the exact shipped image, with remaining motion gates visible in the GUI.

## 5. Validation and agreed defaults

- Run native tests, Clippy, WASM checks, source-policy checks, and release builds for both default and `examples` configurations.
- Add focused tests for fixture-free startup, empty graph creation, tab-state preservation, transactional import failure, and dependency invalidation.
- Update browser qualification to exercise the new visible workflows and verify authenticated simulator state.
- Review layouts at 1440×1000 and 1024×768; the active editor must remain usable with inspectors collapsed.
- Build example assets separately so the embedded release cannot accidentally consume an example-enabled bundle.

RepRap, subtractive machining, ClearPath, and LabVIEW are workflow references. This plan ends at builder essentials and TinyBee bench qualification. Printing/process control, ClearPath integration, physical jogging/homing, loaded motion, and production qualification remain later milestones.
