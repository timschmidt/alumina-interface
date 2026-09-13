//! Canonical hierarchy-derived front-panel input authority and host execution.
//!
//! A saved component panel is presentation metadata until this module resolves
//! an `InputControl` through one exact `ALGH` hierarchy and its independently
//! reproduced flattening. Only the outermost unshadowed control for an
//! otherwise unowned Stream input becomes active. Canonical step-change
//! schedules are expanded to exact samples in the Stream's own clock domain
//! and fed only to the deterministic host simulator; they grant no firmware or
//! deployment authority.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};

use alumina_protocol::Digest;
use alumina_storage::sha256;
use hyperreal::Rational;

use super::analysis::analyze_graph_with_supplied_inputs;
use super::wire::{decode_typed_value_bytes, encode_typed_value_bytes};
use super::{
    CanonicalGraphTrace, GraphAnalysis, GraphAnalysisError, GraphClockId, GraphComponentInputId,
    GraphComponentOutputId, GraphFrontPanelBinding, GraphFrontPanelItemId, GraphFrontPanelRect,
    GraphHierarchyDocument, GraphHierarchyError, GraphHierarchyFlattening,
    GraphHierarchyNodeOrigin, GraphInstanceScope, GraphNodeId, GraphSimulation,
    GraphSimulationError, GraphSimulationHorizon, GraphSimulationLimits, GraphSimulationRegistry,
    GraphTraceEntry, GraphTraceError, GraphTypeId, GraphWireError, GraphWireId,
    InjectedInputSample, TypeKind, TypedGraphValue, WireEndpoint, encode_graph_hierarchy,
    encode_graph_trace, graph_component_instance_input_port, graph_component_instance_output_port,
    simulate_graph_with_inputs,
};

/// Magic bytes at the beginning of every canonical front-panel run document.
pub const GRAPH_FRONT_PANEL_RUN_MAGIC: [u8; 4] = *b"ALFR";

/// Exact canonical front-panel run format implemented by this source tree.
pub const GRAPH_FRONT_PANEL_RUN_VERSION: u16 = 1;

const GRAPH_FRONT_PANEL_RUN_FLAGS: u16 = 0;

/// Allocation and execution ceilings embedded in every `ALFR` document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphFrontPanelRuntimeLimits {
    /// Maximum canonical run-document bytes.
    pub maximum_document_bytes: usize,
    /// Maximum resolved input controls across all component occurrences.
    pub maximum_controls: usize,
    /// Maximum root-to-nested occurrence path depth.
    pub maximum_path_depth: usize,
    /// Maximum retained change points for one control.
    pub maximum_changes_per_control: usize,
    /// Maximum retained change points across the complete run.
    pub maximum_total_changes: usize,
    /// Maximum expanded exact input samples across the complete run.
    pub maximum_expanded_samples: usize,
    /// Maximum inclusive root-clock horizon.
    pub maximum_root_ticks: u64,
}

impl GraphFrontPanelRuntimeLimits {
    /// Bounded first-release browser/WASM policy.
    pub const fn interactive() -> Self {
        Self {
            maximum_document_bytes: 16 * 1024 * 1024,
            maximum_controls: 4_096,
            maximum_path_depth: 64,
            maximum_changes_per_control: 4_096,
            maximum_total_changes: 65_536,
            maximum_expanded_samples: 65_536,
            maximum_root_ticks: 1_000_000,
        }
    }

    fn validate(self) -> Result<(), GraphFrontPanelRuntimeError> {
        if self.maximum_document_bytes == 0
            || self.maximum_controls == 0
            || self.maximum_path_depth == 0
            || self.maximum_changes_per_control == 0
            || self.maximum_total_changes == 0
            || self.maximum_expanded_samples == 0
            || self.maximum_root_ticks == 0
        {
            Err(GraphFrontPanelRuntimeError::ZeroLimit)
        } else {
            Ok(())
        }
    }
}

impl Default for GraphFrontPanelRuntimeLimits {
    fn default() -> Self {
        Self::interactive()
    }
}

/// Stable authority key for one panel item on one component occurrence.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct GraphFrontPanelControlKey {
    source_path: Vec<GraphNodeId>,
    item: GraphFrontPanelItemId,
}

impl GraphFrontPanelControlKey {
    /// Construct an occurrence-local panel-item key.
    pub fn new(source_path: Vec<GraphNodeId>, item: GraphFrontPanelItemId) -> Self {
        Self { source_path, item }
    }

    /// Borrow the root instance followed by nested component-local instances.
    pub fn source_path(&self) -> &[GraphNodeId] {
        &self.source_path
    }

    /// Return the stable panel item inside the final component occurrence.
    pub const fn item(&self) -> GraphFrontPanelItemId {
        self.item
    }
}

/// Stable occurrence-local identity of one public component output.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct GraphFrontPanelOutputKey {
    source_path: Vec<GraphNodeId>,
    output: GraphComponentOutputId,
}

impl GraphFrontPanelOutputKey {
    /// Construct one root-to-nested occurrence plus stable public-output key.
    pub fn new(source_path: Vec<GraphNodeId>, output: GraphComponentOutputId) -> Self {
        Self {
            source_path,
            output,
        }
    }

    /// Borrow the root instance followed by nested component-local instances.
    pub fn source_path(&self) -> &[GraphNodeId] {
        &self.source_path
    }

    /// Return the stable public output inside the selected occurrence.
    pub const fn output(&self) -> GraphComponentOutputId {
        self.output
    }
}

/// Exact flattened Stream authority behind one occurrence-local public output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphFrontPanelOutputAuthority {
    key: GraphFrontPanelOutputKey,
    component: Digest,
    name: String,
    flattened_output: WireEndpoint,
    stream_type: GraphTypeId,
    sample_type: GraphTypeId,
    clock: GraphClockId,
}

impl GraphFrontPanelOutputAuthority {
    /// Borrow the stable occurrence/output identity.
    pub const fn key(&self) -> &GraphFrontPanelOutputKey {
        &self.key
    }

    /// Return the exact component definition containing the public output.
    pub const fn component(&self) -> Digest {
        self.component
    }

    /// Borrow the stable public-output name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the final ordinary Stream output reached after flattening.
    pub const fn flattened_output(&self) -> WireEndpoint {
        self.flattened_output
    }

    /// Return the registered Stream type at the flattened output.
    pub const fn stream_type(&self) -> GraphTypeId {
        self.stream_type
    }

    /// Return the exact scalar/composite sample type carried by the Stream.
    pub const fn sample_type(&self) -> GraphTypeId {
        self.sample_type
    }

    /// Return the Stream's exact clock authority.
    pub const fn clock(&self) -> GraphClockId {
        self.clock
    }
}

/// Why one hierarchy-resolved input control is or is not authoritative.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphFrontPanelInputDisposition {
    /// This control is the sole admitted supplier for its flattened input.
    Active,
    /// An authored structural wire already owns the flattened input.
    Connected(GraphWireId),
    /// A shallower occurrence control resolves to the same final input and wins.
    Superseded(GraphFrontPanelControlKey),
}

/// One exact hierarchy-derived input-control resolution record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphFrontPanelInputAuthority {
    key: GraphFrontPanelControlKey,
    component: Digest,
    input: GraphComponentInputId,
    name: String,
    rect: GraphFrontPanelRect,
    flattened_input: WireEndpoint,
    stream_type: GraphTypeId,
    sample_type: GraphTypeId,
    clock: GraphClockId,
    disposition: GraphFrontPanelInputDisposition,
}

impl GraphFrontPanelInputAuthority {
    /// Borrow the occurrence-local control identity.
    pub const fn key(&self) -> &GraphFrontPanelControlKey {
        &self.key
    }

    /// Return the exact component definition containing the panel item.
    pub const fn component(&self) -> Digest {
        self.component
    }

    /// Return the component-local public input bound by the panel item.
    pub const fn input(&self) -> GraphComponentInputId {
        self.input
    }

    /// Borrow the stable user-facing panel item name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the presentation-only logical panel rectangle.
    pub const fn rect(&self) -> GraphFrontPanelRect {
        self.rect
    }

    /// Return the final ordinary graph input reached after flattening.
    pub const fn flattened_input(&self) -> WireEndpoint {
        self.flattened_input
    }

    /// Return the registered Stream type at the flattened input.
    pub const fn stream_type(&self) -> GraphTypeId {
        self.stream_type
    }

    /// Return the exact scalar/composite sample type accepted by the Stream.
    pub const fn sample_type(&self) -> GraphTypeId {
        self.sample_type
    }

    /// Return the Stream's exact clock authority.
    pub const fn clock(&self) -> GraphClockId {
        self.clock
    }

    /// Borrow the fail-closed authority disposition.
    pub const fn disposition(&self) -> &GraphFrontPanelInputDisposition {
        &self.disposition
    }

    /// Return whether this control may supply host-simulation samples.
    pub const fn is_active(&self) -> bool {
        matches!(self.disposition, GraphFrontPanelInputDisposition::Active)
    }
}

/// Resolve every component-occurrence `InputControl` to final graph authority.
pub fn resolve_graph_front_panel_inputs(
    hierarchy: &GraphHierarchyDocument,
    flattening: &GraphHierarchyFlattening,
    limits: GraphFrontPanelRuntimeLimits,
) -> Result<Vec<GraphFrontPanelInputAuthority>, GraphFrontPanelRuntimeError> {
    limits.validate()?;
    let hierarchy_digest = encode_graph_hierarchy(hierarchy)?.digest();
    if flattening.source_digest() != hierarchy_digest {
        return Err(GraphFrontPanelRuntimeError::HierarchyDigestMismatch);
    }

    let mut authorities = Vec::new();
    for occurrence in flattening.instances() {
        if occurrence.source_path().len() > limits.maximum_path_depth {
            return Err(GraphFrontPanelRuntimeError::LimitExceeded(
                "component occurrence path depth",
            ));
        }
        let component = hierarchy
            .dependency(occurrence.component())
            .ok_or(GraphFrontPanelRuntimeError::HierarchyInvariant)?
            .document();
        for item in component.panel_items() {
            let GraphFrontPanelBinding::InputControl(input) = item.binding() else {
                continue;
            };
            if authorities.len() >= limits.maximum_controls {
                return Err(GraphFrontPanelRuntimeError::LimitExceeded(
                    "resolved input control count",
                ));
            }
            let public_input = component
                .input(input)
                .ok_or(GraphFrontPanelRuntimeError::HierarchyInvariant)?;
            let flattened_input = resolve_flattened_input(
                hierarchy,
                flattening,
                occurrence.component(),
                occurrence.source_path(),
                public_input.target(),
                limits.maximum_path_depth,
            )?;
            let flattened_node = flattening
                .workspace()
                .graph()
                .node(flattened_input.node)
                .ok_or(GraphFrontPanelRuntimeError::HierarchyInvariant)?;
            let port = flattened_node
                .inputs()
                .iter()
                .find(|port| port.id() == flattened_input.port)
                .ok_or(GraphFrontPanelRuntimeError::HierarchyInvariant)?;
            let (sample_type, clock) = match flattening
                .workspace()
                .graph()
                .schema()
                .value_type(port.value_type())
                .map(super::TypeDefinition::kind)
            {
                Some(TypeKind::Stream { sample, clock, .. }) => (*sample, *clock),
                _ => {
                    return Err(GraphFrontPanelRuntimeError::NonStreamControl(
                        GraphFrontPanelControlKey::new(
                            occurrence.source_path().to_vec(),
                            item.id(),
                        ),
                    ));
                }
            };
            if component.input_value_type(input) != Some(port.value_type()) {
                return Err(GraphFrontPanelRuntimeError::HierarchyInvariant);
            }
            let disposition = flattening
                .workspace()
                .graph()
                .wires()
                .iter()
                .find(|wire| wire.target() == flattened_input)
                .map_or(GraphFrontPanelInputDisposition::Active, |wire| {
                    GraphFrontPanelInputDisposition::Connected(wire.id())
                });
            authorities.push(GraphFrontPanelInputAuthority {
                key: GraphFrontPanelControlKey::new(occurrence.source_path().to_vec(), item.id()),
                component: occurrence.component(),
                input,
                name: item.name().to_owned(),
                rect: item.rect(),
                flattened_input,
                stream_type: port.value_type(),
                sample_type,
                clock,
                disposition,
            });
        }
    }
    authorities.sort_unstable_by(|left, right| left.key.cmp(&right.key));

    let mut winners: BTreeMap<WireEndpoint, usize> = BTreeMap::new();
    for (index, authority) in authorities.iter().enumerate() {
        if !authority.is_active() {
            continue;
        }
        winners
            .entry(authority.flattened_input)
            .and_modify(|winner| {
                let current_rank = (
                    authority.key.source_path.len(),
                    &authority.key.source_path,
                    authority.key.item,
                );
                let winner_authority = &authorities[*winner];
                let winner_rank = (
                    winner_authority.key.source_path.len(),
                    &winner_authority.key.source_path,
                    winner_authority.key.item,
                );
                if current_rank < winner_rank {
                    *winner = index;
                }
            })
            .or_insert(index);
    }
    let superseded_by = authorities
        .iter()
        .enumerate()
        .map(|(index, authority)| {
            authority.is_active().then(|| {
                winners
                    .get(&authority.flattened_input)
                    .copied()
                    .filter(|winner| *winner != index)
                    .map(|winner| authorities[winner].key.clone())
            })
        })
        .collect::<Vec<_>>();
    for (authority, winner) in authorities.iter_mut().zip(superseded_by) {
        if let Some(Some(winner)) = winner {
            authority.disposition = GraphFrontPanelInputDisposition::Superseded(winner);
        }
    }
    Ok(authorities)
}

fn resolve_flattened_input(
    hierarchy: &GraphHierarchyDocument,
    flattening: &GraphHierarchyFlattening,
    mut component: Digest,
    source_path: &[GraphNodeId],
    mut target: WireEndpoint,
    maximum_path_depth: usize,
) -> Result<WireEndpoint, GraphFrontPanelRuntimeError> {
    let mut path = source_path.to_vec();
    loop {
        let nested = hierarchy.instances().iter().find(|instance| {
            instance.scope() == GraphInstanceScope::Component(component)
                && instance.node() == target.node
        });
        let Some(nested) = nested else {
            let origin = GraphHierarchyNodeOrigin::Component {
                source_path: path,
                component,
                node: target.node,
            };
            let node = flattening
                .flattened_node(&origin)
                .ok_or(GraphFrontPanelRuntimeError::HierarchyInvariant)?;
            return Ok(WireEndpoint {
                node,
                port: target.port,
            });
        };
        path.push(target.node);
        if path.len() > maximum_path_depth {
            return Err(GraphFrontPanelRuntimeError::LimitExceeded(
                "component occurrence path depth",
            ));
        }
        component = nested.component();
        let child = hierarchy
            .dependency(component)
            .ok_or(GraphFrontPanelRuntimeError::HierarchyInvariant)?
            .document();
        let input = child
            .inputs()
            .iter()
            .find(|input| {
                graph_component_instance_input_port(child, input.id()) == Some(target.port)
            })
            .ok_or(GraphFrontPanelRuntimeError::HierarchyInvariant)?;
        target = input.target();
    }
}

/// Resolve one occurrence-local public output through nested placeholders.
pub fn resolve_graph_front_panel_output(
    hierarchy: &GraphHierarchyDocument,
    flattening: &GraphHierarchyFlattening,
    key: GraphFrontPanelOutputKey,
    limits: GraphFrontPanelRuntimeLimits,
) -> Result<GraphFrontPanelOutputAuthority, GraphFrontPanelRuntimeError> {
    limits.validate()?;
    if flattening.source_digest() != encode_graph_hierarchy(hierarchy)?.digest() {
        return Err(GraphFrontPanelRuntimeError::HierarchyDigestMismatch);
    }
    if key.source_path.is_empty() || key.source_path.len() > limits.maximum_path_depth {
        return Err(GraphFrontPanelRuntimeError::UnknownOutput(key));
    }
    let occurrence = flattening
        .instances()
        .iter()
        .find(|occurrence| occurrence.source_path() == key.source_path)
        .ok_or_else(|| GraphFrontPanelRuntimeError::UnknownOutput(key.clone()))?;
    let component = occurrence.component();
    let document = hierarchy
        .dependency(component)
        .ok_or(GraphFrontPanelRuntimeError::HierarchyInvariant)?
        .document();
    let output = document
        .output(key.output)
        .ok_or_else(|| GraphFrontPanelRuntimeError::UnknownOutput(key.clone()))?;
    let expected_type = document
        .output_value_type(key.output)
        .ok_or(GraphFrontPanelRuntimeError::HierarchyInvariant)?;
    let flattened_output = resolve_flattened_output(
        hierarchy,
        flattening,
        component,
        &key.source_path,
        output.source(),
        limits.maximum_path_depth,
    )?;
    let flattened_node = flattening
        .workspace()
        .graph()
        .node(flattened_output.node)
        .ok_or(GraphFrontPanelRuntimeError::HierarchyInvariant)?;
    let port = flattened_node
        .outputs()
        .iter()
        .find(|port| port.id() == flattened_output.port)
        .ok_or(GraphFrontPanelRuntimeError::HierarchyInvariant)?;
    if port.value_type() != expected_type {
        return Err(GraphFrontPanelRuntimeError::HierarchyInvariant);
    }
    let (sample_type, clock) = match flattening
        .workspace()
        .graph()
        .schema()
        .value_type(port.value_type())
        .map(super::TypeDefinition::kind)
    {
        Some(TypeKind::Stream { sample, clock, .. }) => (*sample, *clock),
        _ => return Err(GraphFrontPanelRuntimeError::NonStreamOutput(key)),
    };
    Ok(GraphFrontPanelOutputAuthority {
        key,
        component,
        name: output.name().to_owned(),
        flattened_output,
        stream_type: port.value_type(),
        sample_type,
        clock,
    })
}

fn resolve_flattened_output(
    hierarchy: &GraphHierarchyDocument,
    flattening: &GraphHierarchyFlattening,
    mut component: Digest,
    source_path: &[GraphNodeId],
    mut source: WireEndpoint,
    maximum_path_depth: usize,
) -> Result<WireEndpoint, GraphFrontPanelRuntimeError> {
    let mut path = source_path.to_vec();
    loop {
        let nested = hierarchy.instances().iter().find(|instance| {
            instance.scope() == GraphInstanceScope::Component(component)
                && instance.node() == source.node
        });
        let Some(nested) = nested else {
            let origin = GraphHierarchyNodeOrigin::Component {
                source_path: path,
                component,
                node: source.node,
            };
            let node = flattening
                .flattened_node(&origin)
                .ok_or(GraphFrontPanelRuntimeError::HierarchyInvariant)?;
            return Ok(WireEndpoint {
                node,
                port: source.port,
            });
        };
        path.push(source.node);
        if path.len() > maximum_path_depth {
            return Err(GraphFrontPanelRuntimeError::LimitExceeded(
                "component occurrence path depth",
            ));
        }
        component = nested.component();
        let child = hierarchy
            .dependency(component)
            .ok_or(GraphFrontPanelRuntimeError::HierarchyInvariant)?
            .document();
        let output = child
            .outputs()
            .iter()
            .find(|output| {
                graph_component_instance_output_port(child, output.id()) == Some(source.port)
            })
            .ok_or(GraphFrontPanelRuntimeError::HierarchyInvariant)?;
        source = output.source();
    }
}

/// One exact value change at a tick in its control Stream's own clock.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphFrontPanelChange {
    clock_tick: u64,
    value: TypedGraphValue,
}

impl GraphFrontPanelChange {
    /// Construct one sample-and-hold change point.
    pub fn new(clock_tick: u64, value: TypedGraphValue) -> Self {
        Self { clock_tick, value }
    }

    /// Return the local Stream clock tick at which this value becomes active.
    pub const fn clock_tick(&self) -> u64 {
        self.clock_tick
    }

    /// Borrow the exact value retained from this tick onward.
    pub const fn value(&self) -> &TypedGraphValue {
        &self.value
    }
}

/// Canonical sample-and-hold schedule for one active input control.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphFrontPanelSchedule {
    control: GraphFrontPanelControlKey,
    changes: Vec<GraphFrontPanelChange>,
}

impl GraphFrontPanelSchedule {
    /// Construct a schedule. Complete run validation canonicalizes and checks it.
    pub fn new(control: GraphFrontPanelControlKey, changes: Vec<GraphFrontPanelChange>) -> Self {
        Self { control, changes }
    }

    /// Borrow the occurrence-local control identity.
    pub const fn control(&self) -> &GraphFrontPanelControlKey {
        &self.control
    }

    /// Borrow change points in canonical increasing local-tick order.
    pub fn changes(&self) -> &[GraphFrontPanelChange] {
        &self.changes
    }
}

/// Canonical executable host front-panel program bound to exact graph context.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphFrontPanelRunDocument {
    limits: GraphFrontPanelRuntimeLimits,
    hierarchy_digest: Digest,
    workspace_digest: Digest,
    registry_digest: Digest,
    horizon: GraphSimulationHorizon,
    schedules: Vec<GraphFrontPanelSchedule>,
}

impl GraphFrontPanelRunDocument {
    /// Validate, canonicalize, and bind one complete front-panel run.
    pub fn try_new(
        hierarchy: &GraphHierarchyDocument,
        flattening: &GraphHierarchyFlattening,
        registry: &GraphSimulationRegistry,
        horizon: GraphSimulationHorizon,
        mut schedules: Vec<GraphFrontPanelSchedule>,
        limits: GraphFrontPanelRuntimeLimits,
    ) -> Result<Self, GraphFrontPanelRuntimeError> {
        limits.validate()?;
        if horizon.inclusive_root_tick() > limits.maximum_root_ticks {
            return Err(GraphFrontPanelRuntimeError::LimitExceeded(
                "root tick horizon",
            ));
        }
        let authorities = resolve_graph_front_panel_inputs(hierarchy, flattening, limits)?;
        let active = authorities
            .iter()
            .filter(|authority| authority.is_active())
            .map(|authority| (authority.key.clone(), authority))
            .collect::<BTreeMap<_, _>>();
        if schedules.len() > limits.maximum_controls {
            return Err(GraphFrontPanelRuntimeError::LimitExceeded(
                "control schedule count",
            ));
        }
        schedules.sort_unstable_by(|left, right| left.control.cmp(&right.control));
        for pair in schedules.windows(2) {
            if pair[0].control == pair[1].control {
                return Err(GraphFrontPanelRuntimeError::DuplicateControl(
                    pair[0].control.clone(),
                ));
            }
        }
        for schedule in &mut schedules {
            if schedule.control.source_path.len() > limits.maximum_path_depth {
                return Err(GraphFrontPanelRuntimeError::LimitExceeded(
                    "control schedule path depth",
                ));
            }
            if !active.contains_key(&schedule.control) {
                return Err(GraphFrontPanelRuntimeError::InactiveControl(
                    schedule.control.clone(),
                ));
            }
            if schedule.changes.is_empty() {
                return Err(GraphFrontPanelRuntimeError::EmptySchedule(
                    schedule.control.clone(),
                ));
            }
            if schedule.changes.len() > limits.maximum_changes_per_control {
                return Err(GraphFrontPanelRuntimeError::LimitExceeded(
                    "changes per control",
                ));
            }
            schedule
                .changes
                .sort_unstable_by_key(GraphFrontPanelChange::clock_tick);
            if schedule.changes[0].clock_tick != 0 {
                return Err(GraphFrontPanelRuntimeError::InitialChangeNotZero(
                    schedule.control.clone(),
                ));
            }
            if schedule
                .changes
                .windows(2)
                .any(|pair| pair[0].clock_tick == pair[1].clock_tick)
            {
                return Err(GraphFrontPanelRuntimeError::DuplicateChangeTick(
                    schedule.control.clone(),
                ));
            }
        }
        if active.len() != schedules.len() {
            let missing = active
                .keys()
                .find(|key| {
                    schedules
                        .binary_search_by(|schedule| schedule.control.cmp(key))
                        .is_err()
                })
                .cloned()
                .ok_or(GraphFrontPanelRuntimeError::HierarchyInvariant)?;
            return Err(GraphFrontPanelRuntimeError::MissingControl(missing));
        }
        let total_changes = schedules.iter().try_fold(0_usize, |total, schedule| {
            total.checked_add(schedule.changes.len()).ok_or(
                GraphFrontPanelRuntimeError::LimitExceeded("total control changes"),
            )
        })?;
        if total_changes > limits.maximum_total_changes {
            return Err(GraphFrontPanelRuntimeError::LimitExceeded(
                "total control changes",
            ));
        }

        let graph = flattening.workspace().graph();
        let supplied = active
            .values()
            .map(|authority| authority.flattened_input)
            .collect::<BTreeSet<_>>();
        let analysis =
            analyze_graph_with_supplied_inputs(graph, registry.semantic_registry(), &supplied)?;
        let root_rate = horizon_root_rate(&analysis, horizon)?;
        let mut expanded = 0_usize;
        for schedule in &schedules {
            let authority = active
                .get(&schedule.control)
                .ok_or(GraphFrontPanelRuntimeError::HierarchyInvariant)?;
            for change in &schedule.changes {
                if change.value.value_type() != authority.sample_type
                    || graph.schema().validate_typed_value(&change.value).is_err()
                {
                    return Err(GraphFrontPanelRuntimeError::ValueType {
                        control: schedule.control.clone(),
                        expected: authority.sample_type,
                        received: change.value.value_type(),
                    });
                }
            }
            let ticks = clock_tick_count(
                &analysis,
                root_rate,
                authority.clock,
                horizon,
                limits.maximum_expanded_samples,
            )?;
            if schedule
                .changes
                .last()
                .is_some_and(|change| change.clock_tick >= ticks as u64)
            {
                return Err(GraphFrontPanelRuntimeError::ChangeAfterHorizon(
                    schedule.control.clone(),
                ));
            }
            expanded =
                expanded
                    .checked_add(ticks)
                    .ok_or(GraphFrontPanelRuntimeError::LimitExceeded(
                        "expanded input samples",
                    ))?;
            if expanded > limits.maximum_expanded_samples {
                return Err(GraphFrontPanelRuntimeError::LimitExceeded(
                    "expanded input samples",
                ));
            }
        }

        Ok(Self {
            limits,
            hierarchy_digest: flattening.source_digest(),
            workspace_digest: flattening.encoding().digest(),
            registry_digest: registry.digest(),
            horizon,
            schedules,
        })
    }

    /// Return the embedded allocation policy.
    pub const fn limits(&self) -> GraphFrontPanelRuntimeLimits {
        self.limits
    }

    /// Return the exact source `ALGH` identity.
    pub const fn hierarchy_digest(&self) -> Digest {
        self.hierarchy_digest
    }

    /// Return the exact flattened `ALGW` identity.
    pub const fn workspace_digest(&self) -> Digest {
        self.workspace_digest
    }

    /// Return the exact host simulation registry identity.
    pub const fn registry_digest(&self) -> Digest {
        self.registry_digest
    }

    /// Return the inclusive exact host simulation horizon.
    pub const fn horizon(&self) -> GraphSimulationHorizon {
        self.horizon
    }

    /// Borrow schedules in canonical occurrence/item order.
    pub fn schedules(&self) -> &[GraphFrontPanelSchedule] {
        &self.schedules
    }
}

/// Canonical `ALFR` bytes paired with their SHA-256 identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalGraphFrontPanelRunEncoding {
    bytes: Vec<u8>,
    digest: Digest,
}

impl CanonicalGraphFrontPanelRunEncoding {
    /// Borrow the complete canonical bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return SHA-256 over exactly [`Self::bytes`].
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    /// Consume the carrier and return its bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Encode one validated run relative to its bound flattened graph schema.
pub fn encode_graph_front_panel_run(
    document: &GraphFrontPanelRunDocument,
    flattening: &GraphHierarchyFlattening,
) -> Result<CanonicalGraphFrontPanelRunEncoding, GraphFrontPanelRuntimeError> {
    if document.hierarchy_digest != flattening.source_digest() {
        return Err(GraphFrontPanelRuntimeError::HierarchyDigestMismatch);
    }
    if document.workspace_digest != flattening.encoding().digest() {
        return Err(GraphFrontPanelRuntimeError::WorkspaceDigestMismatch);
    }
    let mut encoder = Encoder::default();
    encoder.bytes(&GRAPH_FRONT_PANEL_RUN_MAGIC);
    encoder.u16(GRAPH_FRONT_PANEL_RUN_VERSION);
    encoder.u16(GRAPH_FRONT_PANEL_RUN_FLAGS);
    encode_limits(&mut encoder, document.limits)?;
    encoder.digest(document.hierarchy_digest);
    encoder.digest(document.workspace_digest);
    encoder.digest(document.registry_digest);
    encoder.u32(document.horizon.root_clock().get());
    encoder.u64(document.horizon.inclusive_root_tick());
    encoder.count(document.schedules.len(), "control schedule count")?;
    for schedule in &document.schedules {
        encoder.count(schedule.control.source_path.len(), "control path depth")?;
        for node in &schedule.control.source_path {
            encoder.u32(node.get());
        }
        encoder.u32(schedule.control.item.get());
        encoder.count(schedule.changes.len(), "control change count")?;
        for change in &schedule.changes {
            encoder.u64(change.clock_tick);
            let value =
                encode_typed_value_bytes(flattening.workspace().graph().schema(), &change.value)?;
            encoder.length_prefixed(&value, "typed control value")?;
        }
    }
    if encoder.0.len() > document.limits.maximum_document_bytes {
        return Err(GraphFrontPanelRuntimeError::LimitExceeded(
            "run document bytes",
        ));
    }
    let digest = sha256(&encoder.0).digest;
    Ok(CanonicalGraphFrontPanelRunEncoding {
        bytes: encoder.0,
        digest,
    })
}

/// Successfully replayed and byte-for-byte verified `ALFR` document.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphFrontPanelRunReplay {
    document: GraphFrontPanelRunDocument,
    encoding: CanonicalGraphFrontPanelRunEncoding,
}

impl GraphFrontPanelRunReplay {
    /// Borrow the reconstructed validated run.
    pub const fn document(&self) -> &GraphFrontPanelRunDocument {
        &self.document
    }

    /// Borrow the verified canonical bytes and identity.
    pub const fn encoding(&self) -> &CanonicalGraphFrontPanelRunEncoding {
        &self.encoding
    }

    /// Consume the replay and return the run document.
    pub fn into_document(self) -> GraphFrontPanelRunDocument {
        self.document
    }
}

/// Decode, validate against exact hierarchy/registry context, and re-encode.
pub fn replay_graph_front_panel_run(
    bytes: &[u8],
    hierarchy: &GraphHierarchyDocument,
    flattening: &GraphHierarchyFlattening,
    registry: &GraphSimulationRegistry,
    admission: GraphFrontPanelRuntimeLimits,
) -> Result<GraphFrontPanelRunReplay, GraphFrontPanelRuntimeError> {
    admission.validate()?;
    if bytes.len() > admission.maximum_document_bytes {
        return Err(GraphFrontPanelRuntimeError::LimitExceeded(
            "admitted run document bytes",
        ));
    }
    let mut decoder = Decoder::new(bytes);
    if decoder.take(GRAPH_FRONT_PANEL_RUN_MAGIC.len())? != GRAPH_FRONT_PANEL_RUN_MAGIC {
        return Err(GraphFrontPanelRuntimeError::InvalidMagic);
    }
    let version = decoder.u16()?;
    if version != GRAPH_FRONT_PANEL_RUN_VERSION {
        return Err(GraphFrontPanelRuntimeError::UnsupportedVersion(version));
    }
    let flags = decoder.u16()?;
    if flags != GRAPH_FRONT_PANEL_RUN_FLAGS {
        return Err(GraphFrontPanelRuntimeError::UnsupportedFlags(flags));
    }
    let limits = decode_limits(&mut decoder)?;
    if !limits_within(limits, admission) {
        return Err(GraphFrontPanelRuntimeError::LimitExceeded(
            "embedded admission limit",
        ));
    }
    limits.validate()?;
    if bytes.len() > limits.maximum_document_bytes {
        return Err(GraphFrontPanelRuntimeError::LimitExceeded(
            "embedded run document bytes",
        ));
    }
    let hierarchy_digest = decoder.digest()?;
    let workspace_digest = decoder.digest()?;
    let registry_digest = decoder.digest()?;
    if hierarchy_digest != flattening.source_digest() {
        return Err(GraphFrontPanelRuntimeError::HierarchyDigestMismatch);
    }
    if workspace_digest != flattening.encoding().digest() {
        return Err(GraphFrontPanelRuntimeError::WorkspaceDigestMismatch);
    }
    if registry_digest != registry.digest() {
        return Err(GraphFrontPanelRuntimeError::RegistryDigestMismatch);
    }
    let horizon = GraphSimulationHorizon::new(GraphClockId::new(decoder.u32()?), decoder.u64()?);
    let count = decoder.count(limits.maximum_controls, "control schedule count")?;
    let mut schedules = Vec::with_capacity(count);
    let mut total_changes = 0_usize;
    for _ in 0..count {
        let path_count = decoder.count(limits.maximum_path_depth, "control path depth")?;
        let mut path = Vec::with_capacity(path_count);
        for _ in 0..path_count {
            path.push(GraphNodeId::new(decoder.u32()?));
        }
        let item = GraphFrontPanelItemId::new(decoder.u32()?);
        let change_count =
            decoder.count(limits.maximum_changes_per_control, "control change count")?;
        total_changes = total_changes.checked_add(change_count).ok_or(
            GraphFrontPanelRuntimeError::LimitExceeded("total control changes"),
        )?;
        if total_changes > limits.maximum_total_changes {
            return Err(GraphFrontPanelRuntimeError::LimitExceeded(
                "total control changes",
            ));
        }
        let mut changes = Vec::with_capacity(change_count);
        for _ in 0..change_count {
            let clock_tick = decoder.u64()?;
            let value_bytes =
                decoder.length_prefixed(limits.maximum_document_bytes, "typed control value")?;
            let value =
                decode_typed_value_bytes(flattening.workspace().graph().schema(), value_bytes)?;
            changes.push(GraphFrontPanelChange::new(clock_tick, value));
        }
        schedules.push(GraphFrontPanelSchedule::new(
            GraphFrontPanelControlKey::new(path, item),
            changes,
        ));
    }
    if !decoder.is_empty() {
        return Err(GraphFrontPanelRuntimeError::TrailingBytes);
    }
    let document = GraphFrontPanelRunDocument::try_new(
        hierarchy, flattening, registry, horizon, schedules, limits,
    )?;
    let encoding = encode_graph_front_panel_run(&document, flattening)?;
    if encoding.bytes() != bytes {
        return Err(GraphFrontPanelRuntimeError::NonCanonical);
    }
    Ok(GraphFrontPanelRunReplay { document, encoding })
}

/// Host result of expanding and executing one exact front-panel run.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphFrontPanelExecution {
    injected_sample_count: usize,
    simulation: GraphSimulation,
    trace: CanonicalGraphTrace,
}

impl GraphFrontPanelExecution {
    /// Return the exact number of expanded input samples.
    pub const fn injected_sample_count(&self) -> usize {
        self.injected_sample_count
    }

    /// Borrow the deterministic host simulation.
    pub const fn simulation(&self) -> &GraphSimulation {
        &self.simulation
    }

    /// Borrow the independently replayable canonical `ALGT` trace.
    pub const fn trace(&self) -> &CanonicalGraphTrace {
        &self.trace
    }
}

/// One exact public-output sample selected on the simulation root clock.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphFrontPanelOutputSample<'a> {
    authority: GraphFrontPanelOutputAuthority,
    root_tick: Rational,
    entry: &'a GraphTraceEntry,
}

impl<'a> GraphFrontPanelOutputSample<'a> {
    /// Borrow the hierarchy-resolved public-output authority.
    pub const fn authority(&self) -> &GraphFrontPanelOutputAuthority {
        &self.authority
    }

    /// Borrow the exact sample time in the run's independent root-clock ticks.
    pub const fn root_tick(&self) -> &Rational {
        &self.root_tick
    }

    /// Borrow the exact typed trace entry selected at or before the cursor.
    pub const fn entry(&self) -> &'a GraphTraceEntry {
        self.entry
    }
}

/// Resolve and select one public output at or before an exact root-time cursor.
pub fn sample_graph_front_panel_output_at_or_before<'a>(
    run: &GraphFrontPanelRunDocument,
    hierarchy: &GraphHierarchyDocument,
    flattening: &GraphHierarchyFlattening,
    registry: &GraphSimulationRegistry,
    execution: &'a GraphFrontPanelExecution,
    key: GraphFrontPanelOutputKey,
    cursor_root_tick: &Rational,
) -> Result<Option<GraphFrontPanelOutputSample<'a>>, GraphFrontPanelRuntimeError> {
    if run.hierarchy_digest != flattening.source_digest()
        || run.hierarchy_digest != encode_graph_hierarchy(hierarchy)?.digest()
    {
        return Err(GraphFrontPanelRuntimeError::HierarchyDigestMismatch);
    }
    if run.workspace_digest != flattening.encoding().digest() {
        return Err(GraphFrontPanelRuntimeError::WorkspaceDigestMismatch);
    }
    if run.registry_digest != registry.digest() {
        return Err(GraphFrontPanelRuntimeError::RegistryDigestMismatch);
    }
    let simulation = execution.simulation();
    if simulation.graph_digest() != flattening.workspace().graph_digest() {
        return Err(GraphFrontPanelRuntimeError::ExecutionContextMismatch(
            "flattened graph",
        ));
    }
    if simulation.registry_digest() != run.registry_digest {
        return Err(GraphFrontPanelRuntimeError::ExecutionContextMismatch(
            "simulation registry",
        ));
    }
    if simulation.horizon() != run.horizon {
        return Err(GraphFrontPanelRuntimeError::ExecutionContextMismatch(
            "simulation horizon",
        ));
    }
    let maximum_root_tick = Rational::from(run.horizon.inclusive_root_tick());
    if cursor_root_tick < &Rational::from(0) || cursor_root_tick > &maximum_root_tick {
        return Err(GraphFrontPanelRuntimeError::CursorOutsideHorizon {
            cursor: cursor_root_tick.clone(),
            maximum: run.horizon.inclusive_root_tick(),
        });
    }

    let authority = resolve_graph_front_panel_output(hierarchy, flattening, key, run.limits)?;
    let input_authorities = resolve_graph_front_panel_inputs(hierarchy, flattening, run.limits)?;
    let supplied = input_authorities
        .iter()
        .filter(|input| input.is_active())
        .map(|input| input.flattened_input)
        .collect::<BTreeSet<_>>();
    let analysis = analyze_graph_with_supplied_inputs(
        flattening.workspace().graph(),
        registry.semantic_registry(),
        &supplied,
    )?;
    let root_rate = horizon_root_rate(&analysis, run.horizon)?;
    let mut selected: Option<(&GraphTraceEntry, Rational)> = None;
    for entry in simulation
        .entries()
        .iter()
        .filter(|entry| entry.endpoint() == authority.flattened_output)
    {
        if entry.clock() != authority.clock || entry.value().value_type() != authority.sample_type {
            return Err(GraphFrontPanelRuntimeError::OutputSampleMismatch(
                authority.key.clone(),
            ));
        }
        let root_tick =
            clock_tick_on_root(&analysis, root_rate, entry.clock(), entry.clock_tick())?;
        if &root_tick > cursor_root_tick {
            continue;
        }
        let replace = selected.as_ref().is_none_or(|(current, current_root)| {
            root_tick > *current_root
                || (root_tick == *current_root && entry.sequence() > current.sequence())
        });
        if replace {
            selected = Some((entry, root_tick));
        }
    }
    Ok(
        selected.map(|(entry, root_tick)| GraphFrontPanelOutputSample {
            authority,
            root_tick,
            entry,
        }),
    )
}

/// Expand sample-and-hold controls and execute the bound flattened graph.
pub fn execute_graph_front_panel_run(
    run: &GraphFrontPanelRunDocument,
    hierarchy: &GraphHierarchyDocument,
    flattening: &GraphHierarchyFlattening,
    registry: &GraphSimulationRegistry,
    external_samples: &[super::ExternalStreamSample],
    simulation_limits: GraphSimulationLimits,
) -> Result<GraphFrontPanelExecution, GraphFrontPanelRuntimeError> {
    if run.hierarchy_digest != flattening.source_digest()
        || run.hierarchy_digest != encode_graph_hierarchy(hierarchy)?.digest()
    {
        return Err(GraphFrontPanelRuntimeError::HierarchyDigestMismatch);
    }
    if run.workspace_digest != flattening.encoding().digest() {
        return Err(GraphFrontPanelRuntimeError::WorkspaceDigestMismatch);
    }
    if run.registry_digest != registry.digest() {
        return Err(GraphFrontPanelRuntimeError::RegistryDigestMismatch);
    }
    let authorities = resolve_graph_front_panel_inputs(hierarchy, flattening, run.limits)?;
    let active = authorities
        .iter()
        .filter(|authority| authority.is_active())
        .map(|authority| (authority.key.clone(), authority))
        .collect::<BTreeMap<_, _>>();
    let supplied = active
        .values()
        .map(|authority| authority.flattened_input)
        .collect::<BTreeSet<_>>();
    let analysis = analyze_graph_with_supplied_inputs(
        flattening.workspace().graph(),
        registry.semantic_registry(),
        &supplied,
    )?;
    let root_rate = horizon_root_rate(&analysis, run.horizon)?;
    let mut injected = Vec::new();
    for schedule in &run.schedules {
        let authority = active.get(&schedule.control).ok_or_else(|| {
            GraphFrontPanelRuntimeError::InactiveControl(schedule.control.clone())
        })?;
        let ticks = clock_tick_count(
            &analysis,
            root_rate,
            authority.clock,
            run.horizon,
            run.limits.maximum_expanded_samples,
        )?;
        let mut change_index = 0_usize;
        for tick in 0..u64::try_from(ticks)
            .map_err(|_| GraphFrontPanelRuntimeError::IntegerOverflow("expanded tick count"))?
        {
            while change_index + 1 < schedule.changes.len()
                && schedule.changes[change_index + 1].clock_tick <= tick
            {
                change_index += 1;
            }
            if injected.len() >= run.limits.maximum_expanded_samples {
                return Err(GraphFrontPanelRuntimeError::LimitExceeded(
                    "expanded input samples",
                ));
            }
            injected.push(InjectedInputSample::new(
                authority.flattened_input,
                tick,
                tick,
                schedule.changes[change_index].value.clone(),
            ));
        }
    }
    let simulation = simulate_graph_with_inputs(
        flattening.workspace().graph(),
        registry,
        run.horizon,
        external_samples,
        &injected,
        simulation_limits,
    )?;
    let trace = encode_graph_trace(
        flattening.workspace().graph(),
        &simulation,
        simulation_limits,
    )?;
    Ok(GraphFrontPanelExecution {
        injected_sample_count: injected.len(),
        simulation,
        trace,
    })
}

fn horizon_root_rate(
    analysis: &GraphAnalysis,
    horizon: GraphSimulationHorizon,
) -> Result<&super::GraphClockRate, GraphFrontPanelRuntimeError> {
    analysis
        .clock_rate(horizon.root_clock())
        .filter(|rate| rate.root() == horizon.root_clock())
        .ok_or(GraphFrontPanelRuntimeError::InvalidHorizonRoot(
            horizon.root_clock(),
        ))
}

fn clock_tick_count(
    analysis: &GraphAnalysis,
    root_rate: &super::GraphClockRate,
    clock: GraphClockId,
    horizon: GraphSimulationHorizon,
    maximum: usize,
) -> Result<usize, GraphFrontPanelRuntimeError> {
    let horizon_time = Rational::from(horizon.inclusive_root_tick());
    let mut tick = 0_u64;
    let mut count = 0_usize;
    loop {
        let time = clock_tick_on_root(analysis, root_rate, clock, tick)?;
        if time > horizon_time {
            return Ok(count);
        }
        if count >= maximum {
            return Err(GraphFrontPanelRuntimeError::LimitExceeded(
                "expanded input samples",
            ));
        }
        count += 1;
        tick = tick
            .checked_add(1)
            .ok_or(GraphFrontPanelRuntimeError::IntegerOverflow(
                "expanded clock tick",
            ))?;
    }
}

fn clock_tick_on_root(
    analysis: &GraphAnalysis,
    root_rate: &super::GraphClockRate,
    clock: GraphClockId,
    tick: u64,
) -> Result<Rational, GraphFrontPanelRuntimeError> {
    let rate = analysis
        .clock_rate(clock)
        .ok_or(GraphFrontPanelRuntimeError::UnknownClock(clock))?;
    if rate.root() != root_rate.clock() {
        return Err(GraphFrontPanelRuntimeError::ClockOutsideHorizon {
            clock,
            clock_root: rate.root(),
            horizon_root: root_rate.clock(),
        });
    }
    Ok(Rational::from(tick) * root_rate.ticks_per_second().clone()
        / rate.ticks_per_second().clone())
}

/// Rejection at front-panel authority, canonical replay, or host execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphFrontPanelRuntimeError {
    /// One runtime policy limit was zero.
    ZeroLimit,
    /// A bounded collection, byte count, or horizon exceeded policy.
    LimitExceeded(&'static str),
    /// Input did not begin with [`GRAPH_FRONT_PANEL_RUN_MAGIC`].
    InvalidMagic,
    /// The run format version is unsupported.
    UnsupportedVersion(u16),
    /// Reserved run flags were nonzero.
    UnsupportedFlags(u16),
    /// A fixed-width or length-delimited field ran past input.
    Truncated,
    /// A count or length did not fit its canonical integer width.
    IntegerOverflow(&'static str),
    /// Valid fields remained after the canonical document.
    TrailingBytes,
    /// Decoding and canonical reconstruction changed at least one byte.
    NonCanonical,
    /// The flattening was not produced from the supplied hierarchy.
    HierarchyDigestMismatch,
    /// The run names a different flattened workspace.
    WorkspaceDigestMismatch,
    /// The run names a different semantic/implementation registry.
    RegistryDigestMismatch,
    /// Validated hierarchy/flattening facts could not resolve consistently.
    HierarchyInvariant,
    /// No exact component occurrence/public-output authority matched the key.
    UnknownOutput(GraphFrontPanelOutputKey),
    /// A public output did not ultimately resolve to a Stream output.
    NonStreamOutput(GraphFrontPanelOutputKey),
    /// Typed execution evidence did not match the run context.
    ExecutionContextMismatch(&'static str),
    /// An exact root-time cursor fell before zero or after the run horizon.
    CursorOutsideHorizon {
        /// Rejected exact cursor in root-clock ticks.
        cursor: Rational,
        /// Inclusive maximum root-clock tick.
        maximum: u64,
    },
    /// A trace entry contradicted its hierarchy-resolved output type or clock.
    OutputSampleMismatch(GraphFrontPanelOutputKey),
    /// A panel input control did not ultimately target a Stream input.
    NonStreamControl(GraphFrontPanelControlKey),
    /// Two schedules named one exact occurrence/item authority.
    DuplicateControl(GraphFrontPanelControlKey),
    /// A schedule named a connected, superseded, or unknown control.
    InactiveControl(GraphFrontPanelControlKey),
    /// One active input control had no exact schedule.
    MissingControl(GraphFrontPanelControlKey),
    /// One control schedule retained no initial or later value.
    EmptySchedule(GraphFrontPanelControlKey),
    /// Sample-and-hold authority did not begin at local clock tick zero.
    InitialChangeNotZero(GraphFrontPanelControlKey),
    /// Two change points claimed one local clock tick.
    DuplicateChangeTick(GraphFrontPanelControlKey),
    /// A change point fell beyond the inclusive run horizon.
    ChangeAfterHorizon(GraphFrontPanelControlKey),
    /// A control value did not match the resolved Stream sample type.
    ValueType {
        /// Exact occurrence/item authority.
        control: GraphFrontPanelControlKey,
        /// Required Stream sample type.
        expected: GraphTypeId,
        /// Supplied exact graph type.
        received: GraphTypeId,
    },
    /// The selected simulation horizon was not an independent root clock.
    InvalidHorizonRoot(GraphClockId),
    /// A resolved panel Stream referenced no analyzed clock.
    UnknownClock(GraphClockId),
    /// A panel Stream clock belongs to a different independent time root.
    ClockOutsideHorizon {
        /// Rejected control clock.
        clock: GraphClockId,
        /// Independent root of `clock`.
        clock_root: GraphClockId,
        /// Selected run-horizon root.
        horizon_root: GraphClockId,
    },
    /// Canonical hierarchy validation failed.
    Hierarchy(GraphHierarchyError),
    /// Audited semantic analysis failed.
    Analysis(GraphAnalysisError),
    /// Typed-value canonical encoding or replay failed.
    Wire(GraphWireError),
    /// Deterministic host simulation failed.
    Simulation(GraphSimulationError),
    /// Canonical trace encoding failed.
    Trace(GraphTraceError),
}

impl From<GraphHierarchyError> for GraphFrontPanelRuntimeError {
    fn from(value: GraphHierarchyError) -> Self {
        Self::Hierarchy(value)
    }
}

impl From<GraphAnalysisError> for GraphFrontPanelRuntimeError {
    fn from(value: GraphAnalysisError) -> Self {
        Self::Analysis(value)
    }
}

impl From<GraphWireError> for GraphFrontPanelRuntimeError {
    fn from(value: GraphWireError) -> Self {
        Self::Wire(value)
    }
}

impl From<GraphSimulationError> for GraphFrontPanelRuntimeError {
    fn from(value: GraphSimulationError) -> Self {
        Self::Simulation(value)
    }
}

impl From<GraphTraceError> for GraphFrontPanelRuntimeError {
    fn from(value: GraphTraceError) -> Self {
        Self::Trace(value)
    }
}

impl fmt::Display for GraphFrontPanelRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => formatter.write_str("front-panel runtime policy contains zero"),
            Self::LimitExceeded(name) => write!(formatter, "front-panel {name} exceeds policy"),
            Self::InvalidMagic => formatter.write_str("front-panel run magic is invalid"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "front-panel run version {version} is unsupported"
                )
            }
            Self::UnsupportedFlags(flags) => {
                write!(
                    formatter,
                    "front-panel run flags {flags:#06x} are unsupported"
                )
            }
            Self::Truncated => formatter.write_str("front-panel run is truncated"),
            Self::IntegerOverflow(name) => write!(formatter, "front-panel {name} overflows"),
            Self::TrailingBytes => formatter.write_str("front-panel run has trailing bytes"),
            Self::NonCanonical => formatter.write_str("front-panel run is not canonical"),
            Self::HierarchyDigestMismatch => {
                formatter.write_str("front-panel hierarchy identity does not match")
            }
            Self::WorkspaceDigestMismatch => {
                formatter.write_str("front-panel flattened workspace identity does not match")
            }
            Self::RegistryDigestMismatch => {
                formatter.write_str("front-panel simulation registry identity does not match")
            }
            Self::HierarchyInvariant => {
                formatter.write_str("front-panel hierarchy resolution is inconsistent")
            }
            Self::UnknownOutput(output) => {
                write!(formatter, "front-panel output {output:?} is unknown")
            }
            Self::NonStreamOutput(output) => {
                write!(formatter, "front-panel output {output:?} is not a Stream")
            }
            Self::ExecutionContextMismatch(name) => {
                write!(formatter, "front-panel execution {name} does not match")
            }
            Self::CursorOutsideHorizon { cursor, maximum } => write!(
                formatter,
                "front-panel root cursor {cursor} is outside inclusive horizon 0…{maximum}"
            ),
            Self::OutputSampleMismatch(output) => write!(
                formatter,
                "front-panel output {output:?} sample contradicts its resolved authority"
            ),
            Self::NonStreamControl(control) => {
                write!(
                    formatter,
                    "front-panel control {control:?} is not a Stream input"
                )
            }
            Self::DuplicateControl(control) => {
                write!(formatter, "front-panel control {control:?} is duplicated")
            }
            Self::InactiveControl(control) => {
                write!(formatter, "front-panel control {control:?} is not active")
            }
            Self::MissingControl(control) => {
                write!(formatter, "front-panel control {control:?} has no schedule")
            }
            Self::EmptySchedule(control) => {
                write!(
                    formatter,
                    "front-panel control {control:?} has an empty schedule"
                )
            }
            Self::InitialChangeNotZero(control) => write!(
                formatter,
                "front-panel control {control:?} does not begin at tick zero"
            ),
            Self::DuplicateChangeTick(control) => write!(
                formatter,
                "front-panel control {control:?} has a duplicate change tick"
            ),
            Self::ChangeAfterHorizon(control) => write!(
                formatter,
                "front-panel control {control:?} changes after the run horizon"
            ),
            Self::ValueType {
                control,
                expected,
                received,
            } => write!(
                formatter,
                "front-panel control {control:?} expected {expected:?}, received {received:?}"
            ),
            Self::InvalidHorizonRoot(clock) => {
                write!(
                    formatter,
                    "front-panel horizon clock {clock:?} is not a root"
                )
            }
            Self::UnknownClock(clock) => {
                write!(formatter, "front-panel Stream clock {clock:?} is unknown")
            }
            Self::ClockOutsideHorizon {
                clock,
                clock_root,
                horizon_root,
            } => write!(
                formatter,
                "front-panel clock {clock:?} has root {clock_root:?}, not {horizon_root:?}"
            ),
            Self::Hierarchy(error) => write!(formatter, "front-panel hierarchy failed: {error}"),
            Self::Analysis(error) => write!(formatter, "front-panel analysis failed: {error}"),
            Self::Wire(error) => write!(formatter, "front-panel value failed: {error}"),
            Self::Simulation(error) => write!(formatter, "front-panel simulation failed: {error}"),
            Self::Trace(error) => write!(formatter, "front-panel trace failed: {error}"),
        }
    }
}

impl std::error::Error for GraphFrontPanelRuntimeError {}

fn encode_limits(
    encoder: &mut Encoder,
    limits: GraphFrontPanelRuntimeLimits,
) -> Result<(), GraphFrontPanelRuntimeError> {
    for (value, name) in [
        (limits.maximum_document_bytes, "document byte limit"),
        (limits.maximum_controls, "control limit"),
        (limits.maximum_path_depth, "path depth limit"),
        (
            limits.maximum_changes_per_control,
            "per-control change limit",
        ),
        (limits.maximum_total_changes, "total change limit"),
        (limits.maximum_expanded_samples, "expanded sample limit"),
    ] {
        encoder.u64(
            u64::try_from(value).map_err(|_| GraphFrontPanelRuntimeError::IntegerOverflow(name))?,
        );
    }
    encoder.u64(limits.maximum_root_ticks);
    Ok(())
}

fn decode_limits(
    decoder: &mut Decoder<'_>,
) -> Result<GraphFrontPanelRuntimeLimits, GraphFrontPanelRuntimeError> {
    let mut usize_value = |name| {
        usize::try_from(decoder.u64()?)
            .map_err(|_| GraphFrontPanelRuntimeError::IntegerOverflow(name))
    };
    Ok(GraphFrontPanelRuntimeLimits {
        maximum_document_bytes: usize_value("document byte limit")?,
        maximum_controls: usize_value("control limit")?,
        maximum_path_depth: usize_value("path depth limit")?,
        maximum_changes_per_control: usize_value("per-control change limit")?,
        maximum_total_changes: usize_value("total change limit")?,
        maximum_expanded_samples: usize_value("expanded sample limit")?,
        maximum_root_ticks: decoder.u64()?,
    })
}

const fn limits_within(
    embedded: GraphFrontPanelRuntimeLimits,
    admission: GraphFrontPanelRuntimeLimits,
) -> bool {
    embedded.maximum_document_bytes <= admission.maximum_document_bytes
        && embedded.maximum_controls <= admission.maximum_controls
        && embedded.maximum_path_depth <= admission.maximum_path_depth
        && embedded.maximum_changes_per_control <= admission.maximum_changes_per_control
        && embedded.maximum_total_changes <= admission.maximum_total_changes
        && embedded.maximum_expanded_samples <= admission.maximum_expanded_samples
        && embedded.maximum_root_ticks <= admission.maximum_root_ticks
}

#[derive(Default)]
struct Encoder(Vec<u8>);

impl Encoder {
    fn bytes(&mut self, value: &[u8]) {
        self.0.extend_from_slice(value);
    }

    fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    fn digest(&mut self, value: Digest) {
        self.bytes(&value.0);
    }

    fn count(
        &mut self,
        value: usize,
        name: &'static str,
    ) -> Result<(), GraphFrontPanelRuntimeError> {
        self.u32(
            u32::try_from(value).map_err(|_| GraphFrontPanelRuntimeError::IntegerOverflow(name))?,
        );
        Ok(())
    }

    fn length_prefixed(
        &mut self,
        value: &[u8],
        name: &'static str,
    ) -> Result<(), GraphFrontPanelRuntimeError> {
        self.count(value.len(), name)?;
        self.bytes(value);
        Ok(())
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Decoder<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], GraphFrontPanelRuntimeError> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(GraphFrontPanelRuntimeError::Truncated)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(GraphFrontPanelRuntimeError::Truncated)?;
        self.cursor = end;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, GraphFrontPanelRuntimeError> {
        Ok(u16::from_le_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| GraphFrontPanelRuntimeError::Truncated)?,
        ))
    }

    fn u32(&mut self) -> Result<u32, GraphFrontPanelRuntimeError> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| GraphFrontPanelRuntimeError::Truncated)?,
        ))
    }

    fn u64(&mut self) -> Result<u64, GraphFrontPanelRuntimeError> {
        Ok(u64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| GraphFrontPanelRuntimeError::Truncated)?,
        ))
    }

    fn digest(&mut self) -> Result<Digest, GraphFrontPanelRuntimeError> {
        Ok(Digest(
            self.take(32)?
                .try_into()
                .map_err(|_| GraphFrontPanelRuntimeError::Truncated)?,
        ))
    }

    fn count(
        &mut self,
        maximum: usize,
        name: &'static str,
    ) -> Result<usize, GraphFrontPanelRuntimeError> {
        let value = usize::try_from(self.u32()?)
            .map_err(|_| GraphFrontPanelRuntimeError::IntegerOverflow(name))?;
        if value > maximum {
            Err(GraphFrontPanelRuntimeError::LimitExceeded(name))
        } else {
            Ok(value)
        }
    }

    fn length_prefixed(
        &mut self,
        maximum: usize,
        name: &'static str,
    ) -> Result<&'a [u8], GraphFrontPanelRuntimeError> {
        let length = self.count(maximum, name)?;
        self.take(length)
    }

    fn is_empty(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{
        ChannelFullPolicy, ClockDefinition, ClockKind, ExecutionDomain, ExecutionDomainSet,
        ExternalStreamSample, GraphAnalysisLimits, GraphComponentDocument, GraphComponentInput,
        GraphComponentLimits, GraphComponentOutput, GraphDocument, GraphFrontPanelItem,
        GraphLimits, GraphNodePlacement, GraphNodeRegistry, GraphPortId, GraphSchema,
        GraphSimulationImplementation, GraphSimulationNodeKind, GraphTraceEntryKind, GraphValue,
        GraphWorkspaceDocument, GraphWorkspaceLimits, InputConnectionRequirement, NodeDefinition,
        NodeInputChannelContract, NodeInputChannelKind, NodeKind, NodeOutputDependency, NodeSchema,
        PortDefinition, TypeDefinition, encode_graph_component, flatten_graph_hierarchy,
        graph_component_instance_prototype, replay_graph_trace,
    };

    const ROOT: GraphClockId = GraphClockId::new(1);
    const BOOL: GraphTypeId = GraphTypeId::new(1);
    const STREAM: GraphTypeId = GraphTypeId::new(2);

    fn endpoint(node: u32, port: u32) -> WireEndpoint {
        WireEndpoint {
            node: GraphNodeId::new(node),
            port: GraphPortId::new(port),
        }
    }

    fn schema() -> GraphSchema {
        GraphSchema::try_new(
            GraphLimits::interactive(),
            Vec::new(),
            vec![
                TypeDefinition::new(BOOL, "core.bool", TypeKind::Boolean),
                TypeDefinition::new(
                    STREAM,
                    "stream.bool.root",
                    TypeKind::Stream {
                        sample: BOOL,
                        clock: ROOT,
                        capacity: 8,
                    },
                ),
            ],
        )
        .unwrap()
    }

    fn clocks() -> Vec<ClockDefinition> {
        vec![ClockDefinition::new(
            ROOT,
            "host.root",
            ClockKind::HostMonotonic {
                ticks_per_second: 1_000,
            },
        )]
    }

    fn placeholder_node(
        id: GraphNodeId,
        component: &GraphComponentDocument,
        label: &str,
    ) -> NodeDefinition {
        let prototype = graph_component_instance_prototype(component, label).unwrap();
        NodeDefinition::new(
            id,
            prototype.kind().clone(),
            prototype.label(),
            prototype.domain(),
            prototype.inputs().to_vec(),
            prototype.outputs().to_vec(),
            prototype.parameters().to_vec(),
        )
    }

    fn sink_component() -> GraphComponentDocument {
        let node = NodeDefinition::new(
            GraphNodeId::new(1),
            NodeKind::new("test.stream.sink", 1),
            "Boolean sink",
            ExecutionDomain::HostExact,
            vec![PortDefinition::new(GraphPortId::new(1), "samples", STREAM)],
            Vec::new(),
            Vec::new(),
        );
        let graph = GraphDocument::try_new(1, schema(), clocks(), vec![node], Vec::new()).unwrap();
        let workspace = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            2,
            1,
            graph,
            vec![GraphNodePlacement::new(GraphNodeId::new(1), 20, 20)],
        )
        .unwrap();
        GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            "test.boolean_sink",
            2,
            1,
            2,
            workspace,
            vec![GraphComponentInput::new(
                GraphComponentInputId::new(1),
                "samples",
                endpoint(1, 1),
            )],
            Vec::new(),
            vec![GraphFrontPanelItem::new(
                GraphFrontPanelItemId::new(1),
                "enable",
                GraphFrontPanelBinding::InputControl(GraphComponentInputId::new(1)),
                GraphFrontPanelRect::new(10, 10, 120, 40),
            )],
        )
        .unwrap()
    }

    fn wrapper_component(child: &GraphComponentDocument) -> GraphComponentDocument {
        let placeholder = placeholder_node(GraphNodeId::new(1), child, "Nested sink");
        let graph =
            GraphDocument::try_new(1, schema(), clocks(), vec![placeholder], Vec::new()).unwrap();
        let workspace = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            2,
            1,
            graph,
            vec![GraphNodePlacement::new(GraphNodeId::new(1), 30, 30)],
        )
        .unwrap();
        GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            "test.boolean_sink_wrapper",
            2,
            1,
            2,
            workspace,
            vec![GraphComponentInput::new(
                GraphComponentInputId::new(1),
                "samples",
                endpoint(
                    1,
                    graph_component_instance_input_port(child, GraphComponentInputId::new(1))
                        .unwrap()
                        .get(),
                ),
            )],
            Vec::new(),
            vec![GraphFrontPanelItem::new(
                GraphFrontPanelItemId::new(1),
                "outer_enable",
                GraphFrontPanelBinding::InputControl(GraphComponentInputId::new(1)),
                GraphFrontPanelRect::new(15, 15, 150, 44),
            )],
        )
        .unwrap()
    }

    fn fixture() -> (
        GraphHierarchyDocument,
        GraphHierarchyFlattening,
        GraphSimulationRegistry,
    ) {
        let child = sink_component();
        let child_digest = encode_graph_component(&child).unwrap().digest();
        let wrapper = wrapper_component(&child);
        let wrapper_digest = encode_graph_component(&wrapper).unwrap().digest();
        let placeholder = placeholder_node(GraphNodeId::new(1), &wrapper, "Root wrapper");
        let graph =
            GraphDocument::try_new(1, schema(), clocks(), vec![placeholder], Vec::new()).unwrap();
        let root = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            2,
            1,
            graph,
            vec![GraphNodePlacement::new(GraphNodeId::new(1), 40, 40)],
        )
        .unwrap();
        let hierarchy = GraphHierarchyDocument::try_new(
            super::super::GraphHierarchyLimits::interactive(),
            1,
            root,
            vec![child, wrapper],
            vec![
                super::super::GraphComponentInstance::root(GraphNodeId::new(1), wrapper_digest),
                super::super::GraphComponentInstance::nested(
                    wrapper_digest,
                    GraphNodeId::new(1),
                    child_digest,
                ),
            ],
        )
        .unwrap();
        let flattening = flatten_graph_hierarchy(&hierarchy).unwrap();
        let sink = flattening.workspace().graph().nodes()[0].clone();
        let semantic = GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            flattening.workspace().graph(),
            vec![NodeSchema::new(
                NodeKind::new("test.stream.sink", 1),
                ExecutionDomainSet::HOST_EXACT,
                sink.inputs().to_vec(),
                vec![NodeInputChannelContract::new(
                    GraphPortId::new(1),
                    InputConnectionRequirement::Required,
                    NodeInputChannelKind::StreamQueue {
                        capacity: 4,
                        full_policy: ChannelFullPolicy::Fault,
                    },
                )],
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                None,
            )],
        )
        .unwrap();
        let registry = GraphSimulationRegistry::try_new(
            semantic,
            vec![GraphSimulationImplementation::new(
                NodeKind::new("test.stream.sink", 1),
                GraphSimulationNodeKind::StreamSink {
                    input: GraphPortId::new(1),
                },
            )],
        )
        .unwrap();
        (hierarchy, flattening, registry)
    }

    fn source_sink_component() -> GraphComponentDocument {
        let sink = NodeDefinition::new(
            GraphNodeId::new(1),
            NodeKind::new("test.stream.sink", 1),
            "Boolean sink",
            ExecutionDomain::HostExact,
            vec![PortDefinition::new(GraphPortId::new(1), "samples", STREAM)],
            Vec::new(),
            Vec::new(),
        );
        let source = NodeDefinition::new(
            GraphNodeId::new(2),
            NodeKind::new("test.stream.source", 1),
            "Boolean source",
            ExecutionDomain::HostExact,
            Vec::new(),
            vec![PortDefinition::new(GraphPortId::new(1), "samples", STREAM)],
            Vec::new(),
        );
        let graph =
            GraphDocument::try_new(1, schema(), clocks(), vec![sink, source], Vec::new()).unwrap();
        let workspace = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            3,
            1,
            graph,
            vec![
                GraphNodePlacement::new(GraphNodeId::new(1), 20, 20),
                GraphNodePlacement::new(GraphNodeId::new(2), 180, 20),
            ],
        )
        .unwrap();
        GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            "test.boolean_source_sink",
            2,
            2,
            3,
            workspace,
            vec![GraphComponentInput::new(
                GraphComponentInputId::new(1),
                "sink_samples",
                endpoint(1, 1),
            )],
            vec![GraphComponentOutput::new(
                GraphComponentOutputId::new(1),
                "source_samples",
                endpoint(2, 1),
            )],
            vec![
                GraphFrontPanelItem::new(
                    GraphFrontPanelItemId::new(1),
                    "enable",
                    GraphFrontPanelBinding::InputControl(GraphComponentInputId::new(1)),
                    GraphFrontPanelRect::new(10, 10, 120, 40),
                ),
                GraphFrontPanelItem::new(
                    GraphFrontPanelItemId::new(2),
                    "observed",
                    GraphFrontPanelBinding::OutputIndicator(GraphComponentOutputId::new(1)),
                    GraphFrontPanelRect::new(10, 60, 120, 40),
                ),
            ],
        )
        .unwrap()
    }

    fn source_sink_wrapper(child: &GraphComponentDocument) -> GraphComponentDocument {
        let placeholder = placeholder_node(GraphNodeId::new(1), child, "Nested source/sink");
        let graph =
            GraphDocument::try_new(1, schema(), clocks(), vec![placeholder], Vec::new()).unwrap();
        let workspace = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            2,
            1,
            graph,
            vec![GraphNodePlacement::new(GraphNodeId::new(1), 30, 30)],
        )
        .unwrap();
        GraphComponentDocument::try_new(
            GraphComponentLimits::interactive(),
            1,
            1,
            "test.boolean_source_sink_wrapper",
            2,
            2,
            3,
            workspace,
            vec![GraphComponentInput::new(
                GraphComponentInputId::new(1),
                "sink_samples",
                endpoint(
                    1,
                    graph_component_instance_input_port(child, GraphComponentInputId::new(1))
                        .unwrap()
                        .get(),
                ),
            )],
            vec![GraphComponentOutput::new(
                GraphComponentOutputId::new(1),
                "source_samples",
                endpoint(
                    1,
                    graph_component_instance_output_port(child, GraphComponentOutputId::new(1))
                        .unwrap()
                        .get(),
                ),
            )],
            vec![
                GraphFrontPanelItem::new(
                    GraphFrontPanelItemId::new(1),
                    "outer_enable",
                    GraphFrontPanelBinding::InputControl(GraphComponentInputId::new(1)),
                    GraphFrontPanelRect::new(15, 15, 150, 44),
                ),
                GraphFrontPanelItem::new(
                    GraphFrontPanelItemId::new(2),
                    "outer_observed",
                    GraphFrontPanelBinding::OutputIndicator(GraphComponentOutputId::new(1)),
                    GraphFrontPanelRect::new(15, 70, 150, 44),
                ),
            ],
        )
        .unwrap()
    }

    fn output_fixture() -> (
        GraphHierarchyDocument,
        GraphHierarchyFlattening,
        GraphSimulationRegistry,
    ) {
        let child = source_sink_component();
        let child_digest = encode_graph_component(&child).unwrap().digest();
        let wrapper = source_sink_wrapper(&child);
        let wrapper_digest = encode_graph_component(&wrapper).unwrap().digest();
        let placeholder = placeholder_node(GraphNodeId::new(1), &wrapper, "Root wrapper");
        let graph =
            GraphDocument::try_new(1, schema(), clocks(), vec![placeholder], Vec::new()).unwrap();
        let root = GraphWorkspaceDocument::try_new(
            GraphWorkspaceLimits::interactive(),
            1,
            2,
            1,
            graph,
            vec![GraphNodePlacement::new(GraphNodeId::new(1), 40, 40)],
        )
        .unwrap();
        let hierarchy = GraphHierarchyDocument::try_new(
            super::super::GraphHierarchyLimits::interactive(),
            1,
            root,
            vec![child, wrapper],
            vec![
                super::super::GraphComponentInstance::root(GraphNodeId::new(1), wrapper_digest),
                super::super::GraphComponentInstance::nested(
                    wrapper_digest,
                    GraphNodeId::new(1),
                    child_digest,
                ),
            ],
        )
        .unwrap();
        let flattening = flatten_graph_hierarchy(&hierarchy).unwrap();
        let graph = flattening.workspace().graph();
        let sink = graph
            .nodes()
            .iter()
            .find(|node| node.kind().name() == "test.stream.sink")
            .unwrap();
        let source = graph
            .nodes()
            .iter()
            .find(|node| node.kind().name() == "test.stream.source")
            .unwrap();
        let stream_channel = NodeInputChannelContract::new(
            GraphPortId::new(1),
            InputConnectionRequirement::Required,
            NodeInputChannelKind::StreamQueue {
                capacity: 4,
                full_policy: ChannelFullPolicy::Fault,
            },
        );
        let semantic = GraphNodeRegistry::try_new(
            GraphAnalysisLimits::interactive(),
            graph,
            vec![
                NodeSchema::new(
                    sink.kind().clone(),
                    ExecutionDomainSet::HOST_EXACT,
                    sink.inputs().to_vec(),
                    vec![stream_channel],
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    None,
                ),
                NodeSchema::new(
                    source.kind().clone(),
                    ExecutionDomainSet::HOST_EXACT,
                    Vec::new(),
                    Vec::new(),
                    source.outputs().to_vec(),
                    Vec::new(),
                    vec![NodeOutputDependency::new(GraphPortId::new(1), Vec::new())],
                    Vec::new(),
                    None,
                ),
            ],
        )
        .unwrap();
        let registry = GraphSimulationRegistry::try_new(
            semantic,
            vec![
                GraphSimulationImplementation::new(
                    sink.kind().clone(),
                    GraphSimulationNodeKind::StreamSink {
                        input: GraphPortId::new(1),
                    },
                ),
                GraphSimulationImplementation::new(
                    source.kind().clone(),
                    GraphSimulationNodeKind::ExternalStreamSource {
                        output: GraphPortId::new(1),
                    },
                ),
            ],
        )
        .unwrap();
        (hierarchy, flattening, registry)
    }

    fn boolean(flattening: &GraphHierarchyFlattening, value: bool) -> TypedGraphValue {
        TypedGraphValue::try_new(
            flattening.workspace().graph().schema(),
            BOOL,
            GraphValue::Boolean(value),
        )
        .unwrap()
    }

    #[test]
    fn nested_outer_control_executes_and_replays_canonically() {
        let (hierarchy, flattening, registry) = fixture();
        let limits = GraphFrontPanelRuntimeLimits::interactive();
        let authorities =
            resolve_graph_front_panel_inputs(&hierarchy, &flattening, limits).unwrap();
        assert_eq!(authorities.len(), 2);
        let outer = GraphFrontPanelControlKey::new(
            vec![GraphNodeId::new(1)],
            GraphFrontPanelItemId::new(1),
        );
        let inner = GraphFrontPanelControlKey::new(
            vec![GraphNodeId::new(1), GraphNodeId::new(1)],
            GraphFrontPanelItemId::new(1),
        );
        assert_eq!(authorities[0].key(), &outer);
        assert_eq!(
            authorities[0].disposition(),
            &GraphFrontPanelInputDisposition::Active
        );
        assert_eq!(authorities[1].key(), &inner);
        assert_eq!(
            authorities[1].disposition(),
            &GraphFrontPanelInputDisposition::Superseded(outer.clone())
        );
        assert_eq!(
            authorities[0].flattened_input(),
            authorities[1].flattened_input()
        );

        let run = GraphFrontPanelRunDocument::try_new(
            &hierarchy,
            &flattening,
            &registry,
            GraphSimulationHorizon::new(ROOT, 3),
            vec![GraphFrontPanelSchedule::new(
                outer.clone(),
                vec![
                    GraphFrontPanelChange::new(2, boolean(&flattening, true)),
                    GraphFrontPanelChange::new(0, boolean(&flattening, false)),
                ],
            )],
            limits,
        )
        .unwrap();
        assert_eq!(run.schedules()[0].changes()[0].clock_tick(), 0);

        let encoding = encode_graph_front_panel_run(&run, &flattening).unwrap();
        assert_eq!(encoding.digest(), sha256(encoding.bytes()).digest);
        let replay = replay_graph_front_panel_run(
            encoding.bytes(),
            &hierarchy,
            &flattening,
            &registry,
            limits,
        )
        .unwrap();
        assert_eq!(replay.document(), &run);
        assert_eq!(replay.encoding(), &encoding);

        let execution = execute_graph_front_panel_run(
            replay.document(),
            &hierarchy,
            &flattening,
            &registry,
            &[],
            GraphSimulationLimits::interactive(),
        )
        .unwrap();
        assert_eq!(execution.injected_sample_count(), 4);
        let values = execution
            .simulation()
            .entries()
            .iter()
            .map(|entry| {
                assert_eq!(entry.kind(), GraphTraceEntryKind::InjectedInput);
                match entry.value().value() {
                    GraphValue::Boolean(value) => *value,
                    value => panic!("unexpected front-panel value {value:?}"),
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(values, vec![false, false, true, true]);
        let trace_replay = replay_graph_trace(
            execution.trace().bytes(),
            flattening.workspace().graph(),
            &registry,
            GraphSimulationLimits::interactive(),
        )
        .unwrap();
        assert_eq!(trace_replay.simulation(), execution.simulation());

        for length in 0..encoding.bytes().len() {
            assert!(
                replay_graph_front_panel_run(
                    &encoding.bytes()[..length],
                    &hierarchy,
                    &flattening,
                    &registry,
                    limits,
                )
                .is_err(),
                "strict ALFR prefix {length} unexpectedly replayed"
            );
        }
    }

    #[test]
    fn nested_output_projects_sample_and_hold_at_an_exact_root_cursor() {
        let (hierarchy, flattening, registry) = output_fixture();
        let limits = GraphFrontPanelRuntimeLimits::interactive();
        let control = GraphFrontPanelControlKey::new(
            vec![GraphNodeId::new(1)],
            GraphFrontPanelItemId::new(1),
        );
        let run = GraphFrontPanelRunDocument::try_new(
            &hierarchy,
            &flattening,
            &registry,
            GraphSimulationHorizon::new(ROOT, 3),
            vec![GraphFrontPanelSchedule::new(
                control,
                vec![GraphFrontPanelChange::new(0, boolean(&flattening, false))],
            )],
            limits,
        )
        .unwrap();
        let outer_key = GraphFrontPanelOutputKey::new(
            vec![GraphNodeId::new(1)],
            GraphComponentOutputId::new(1),
        );
        let inner_key = GraphFrontPanelOutputKey::new(
            vec![GraphNodeId::new(1), GraphNodeId::new(1)],
            GraphComponentOutputId::new(1),
        );
        let outer =
            resolve_graph_front_panel_output(&hierarchy, &flattening, outer_key.clone(), limits)
                .unwrap();
        let inner =
            resolve_graph_front_panel_output(&hierarchy, &flattening, inner_key.clone(), limits)
                .unwrap();
        assert_eq!(outer.flattened_output(), inner.flattened_output());
        assert_eq!(outer.sample_type(), BOOL);
        assert_eq!(outer.clock(), ROOT);

        let external = (0_u64..=3)
            .map(|tick| {
                ExternalStreamSample::new(
                    outer.flattened_output(),
                    tick,
                    100 + tick,
                    boolean(&flattening, tick % 2 == 1),
                )
            })
            .collect::<Vec<_>>();
        let execution = execute_graph_front_panel_run(
            &run,
            &hierarchy,
            &flattening,
            &registry,
            &external,
            GraphSimulationLimits::interactive(),
        )
        .unwrap();
        let cursor = Rational::fraction(3, 2).unwrap();
        let selected = sample_graph_front_panel_output_at_or_before(
            &run,
            &hierarchy,
            &flattening,
            &registry,
            &execution,
            outer_key.clone(),
            &cursor,
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected.authority(), &outer);
        assert_eq!(selected.root_tick(), &Rational::from(1));
        assert_eq!(selected.entry().kind(), GraphTraceEntryKind::ExternalSource);
        assert_eq!(selected.entry().clock_tick(), 1);
        assert_eq!(selected.entry().sequence(), 101);
        assert_eq!(selected.entry().value().value(), &GraphValue::Boolean(true));

        assert_eq!(
            sample_graph_front_panel_output_at_or_before(
                &run,
                &hierarchy,
                &flattening,
                &registry,
                &execution,
                inner_key,
                &Rational::fraction(1, 2).unwrap(),
            )
            .unwrap()
            .unwrap()
            .entry()
            .value()
            .value(),
            &GraphValue::Boolean(false),
        );
        let unknown = GraphFrontPanelOutputKey::new(
            vec![GraphNodeId::new(99)],
            GraphComponentOutputId::new(1),
        );
        assert_eq!(
            resolve_graph_front_panel_output(&hierarchy, &flattening, unknown.clone(), limits),
            Err(GraphFrontPanelRuntimeError::UnknownOutput(unknown)),
        );
        assert_eq!(
            sample_graph_front_panel_output_at_or_before(
                &run,
                &hierarchy,
                &flattening,
                &registry,
                &execution,
                outer_key.clone(),
                &Rational::from(-1),
            ),
            Err(GraphFrontPanelRuntimeError::CursorOutsideHorizon {
                cursor: Rational::from(-1),
                maximum: 3,
            }),
        );
        assert_eq!(
            sample_graph_front_panel_output_at_or_before(
                &run,
                &hierarchy,
                &flattening,
                &registry,
                &execution,
                outer_key.clone(),
                &Rational::from(4),
            ),
            Err(GraphFrontPanelRuntimeError::CursorOutsideHorizon {
                cursor: Rational::from(4),
                maximum: 3,
            }),
        );

        let short_run = GraphFrontPanelRunDocument::try_new(
            &hierarchy,
            &flattening,
            &registry,
            GraphSimulationHorizon::new(ROOT, 2),
            run.schedules().to_vec(),
            limits,
        )
        .unwrap();
        let short_execution = execute_graph_front_panel_run(
            &short_run,
            &hierarchy,
            &flattening,
            &registry,
            &external[..3],
            GraphSimulationLimits::interactive(),
        )
        .unwrap();
        assert_eq!(
            sample_graph_front_panel_output_at_or_before(
                &run,
                &hierarchy,
                &flattening,
                &registry,
                &short_execution,
                outer_key,
                &Rational::from(1),
            ),
            Err(GraphFrontPanelRuntimeError::ExecutionContextMismatch(
                "simulation horizon"
            )),
        );
    }

    #[test]
    fn inactive_missing_duplicate_and_late_controls_fail_closed() {
        let (hierarchy, flattening, registry) = fixture();
        let limits = GraphFrontPanelRuntimeLimits::interactive();
        let outer = GraphFrontPanelControlKey::new(
            vec![GraphNodeId::new(1)],
            GraphFrontPanelItemId::new(1),
        );
        let inner = GraphFrontPanelControlKey::new(
            vec![GraphNodeId::new(1), GraphNodeId::new(1)],
            GraphFrontPanelItemId::new(1),
        );
        let horizon = GraphSimulationHorizon::new(ROOT, 3);
        let schedule = |key, tick| {
            GraphFrontPanelSchedule::new(
                key,
                vec![GraphFrontPanelChange::new(
                    tick,
                    boolean(&flattening, false),
                )],
            )
        };

        assert_eq!(
            GraphFrontPanelRunDocument::try_new(
                &hierarchy,
                &flattening,
                &registry,
                horizon,
                Vec::new(),
                limits,
            ),
            Err(GraphFrontPanelRuntimeError::MissingControl(outer.clone()))
        );
        assert_eq!(
            GraphFrontPanelRunDocument::try_new(
                &hierarchy,
                &flattening,
                &registry,
                horizon,
                vec![schedule(inner.clone(), 0)],
                limits,
            ),
            Err(GraphFrontPanelRuntimeError::InactiveControl(inner))
        );
        assert_eq!(
            GraphFrontPanelRunDocument::try_new(
                &hierarchy,
                &flattening,
                &registry,
                horizon,
                vec![schedule(outer.clone(), 0), schedule(outer.clone(), 0)],
                limits,
            ),
            Err(GraphFrontPanelRuntimeError::DuplicateControl(outer.clone()))
        );
        assert_eq!(
            GraphFrontPanelRunDocument::try_new(
                &hierarchy,
                &flattening,
                &registry,
                horizon,
                vec![schedule(outer.clone(), 1)],
                limits,
            ),
            Err(GraphFrontPanelRuntimeError::InitialChangeNotZero(
                outer.clone()
            ))
        );
        assert_eq!(
            GraphFrontPanelRunDocument::try_new(
                &hierarchy,
                &flattening,
                &registry,
                horizon,
                vec![GraphFrontPanelSchedule::new(
                    outer.clone(),
                    vec![
                        GraphFrontPanelChange::new(0, boolean(&flattening, false)),
                        GraphFrontPanelChange::new(4, boolean(&flattening, true)),
                    ],
                )],
                limits,
            ),
            Err(GraphFrontPanelRuntimeError::ChangeAfterHorizon(outer))
        );
    }
}
