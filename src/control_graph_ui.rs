//! Bounded browser/native presentation of the representative exact controller.
//!
//! All graph and trace authority remains in `alumina-interface-core`. This
//! module performs only deterministic layout and named, lossy display
//! projection into egui coordinates.

use std::collections::{BTreeMap, BTreeSet};

use alumina_interface_core::graph::{
    ExecutionDomain, GraphDocument, GraphNodeId, GraphSimulationRegistry, GraphTraceEntryKind,
    GraphValue, GraphWireId, NodeDefinition, RepresentativeControlSignal,
    RepresentativeExactControlGraph, TypeKind, WireEndpoint,
    compile_representative_exact_control_graph,
};
use eframe::egui;

const MAXIMUM_VISIBLE_NODES: usize = 256;
const MAXIMUM_VISIBLE_WIRES: usize = 1_024;
const MAXIMUM_POINTS_PER_SERIES: usize = 4_096;
const NODE_WIDTH: f32 = 218.0;
const COLUMN_GAP: f32 = 82.0;
const NODE_GAP: f32 = 24.0;
const CANVAS_MARGIN: f32 = 28.0;
const NODE_HEADER_HEIGHT: f32 = 48.0;
const PORT_ROW_HEIGHT: f32 = 20.0;
const SIGNALS: [RepresentativeControlSignal; 4] = [
    RepresentativeControlSignal::Error,
    RepresentativeControlSignal::IntegralPrior,
    RepresentativeControlSignal::ClampedController,
    RepresentativeControlSignal::PermittedOutput,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WirePresentation {
    feedback_lane: Option<usize>,
}

#[derive(Clone, Debug)]
struct NodePresentation {
    rect: egui::Rect,
    rank: usize,
}

#[derive(Clone, Debug)]
struct GraphPresentation {
    nodes: BTreeMap<GraphNodeId, NodePresentation>,
    wires: BTreeMap<GraphWireId, WirePresentation>,
    size: egui::Vec2,
}

#[derive(Clone, Debug)]
struct TracePoint {
    tick: u64,
    exact: String,
    enclosure: [f64; 2],
}

#[derive(Clone, Debug)]
struct TraceSeries {
    signal: RepresentativeControlSignal,
    points: Vec<TracePoint>,
}

/// Browser/native inspector for one shared exact control graph and trace.
pub(crate) struct ExactControlWorkspace {
    fixture: RepresentativeExactControlGraph,
    presentation: GraphPresentation,
    traces: Vec<TraceSeries>,
    selected_node: Option<GraphNodeId>,
    cursor_tick: u64,
}

impl ExactControlWorkspace {
    pub(crate) fn try_new() -> Result<Self, String> {
        let fixture =
            compile_representative_exact_control_graph().map_err(|error| error.to_string())?;
        let presentation = graph_presentation(fixture.document(), fixture.registry())?;
        let traces = trace_series(&fixture)?;
        Ok(Self {
            fixture,
            presentation,
            traces,
            selected_node: None,
            cursor_tick: 0,
        })
    }

    pub(crate) fn show_sidebar(&self, ui: &mut egui::Ui) {
        let document = self.fixture.document();
        ui.label("Exact control graph inspector");
        ui.label(format!(
            "Saved nodes / wires: {} / {}",
            document.nodes().len(),
            document.wires().len()
        ));
        ui.label(format!(
            "Canonical trace: {} entries / {} bytes",
            self.fixture.simulation().entries().len(),
            self.fixture.trace().bytes().len()
        ));
        ui.monospace(format!(
            "graph {}…",
            digest_prefix(self.fixture.simulation().graph_digest().0)
        ));
        ui.monospace(format!(
            "registry {}…",
            digest_prefix(self.fixture.simulation().registry_digest().0)
        ));
        ui.monospace(format!(
            "trace {}…",
            digest_prefix(self.fixture.trace().digest().0)
        ));
        ui.colored_label(
            egui::Color32::YELLOW,
            "HostExact simulation only — no firmware or output authority.",
        );
        if let Some(selected) = self.selected_node
            && let Some(node) = document.node(selected)
        {
            ui.separator();
            ui.strong("Selected node");
            ui.label(format!("#{} {}", selected.get(), node.label()));
            ui.monospace(format!("{} v{}", node.kind().name(), node.kind().version()));
        }
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Exact PID / interlock");
            ui.label("50 Hz acquisition to 10 Hz control");
            ui.colored_label(egui::Color32::LIGHT_BLUE, "canonical replay attached");
        });
        ui.label(
            "Inspect the saved typed graph, explicit delay state, feedback wires, and exact trace. Layout and plots are display projections only.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(egui::Color32::from_rgb(96, 169, 232), "— exact Stream");
            ui.colored_label(egui::Color32::from_rgb(241, 178, 84), "— Boolean Stream");
            ui.colored_label(egui::Color32::from_rgb(209, 158, 255), "outlined state");
        });
        ui.separator();

        let graph_height = (ui.available_height() * 0.5).clamp(230.0, 430.0);
        self.show_graph(ui, graph_height);
        self.show_selected_node(ui);
        ui.separator();
        self.show_trace(ui);
    }

    fn show_graph(&mut self, ui: &mut egui::Ui, maximum_height: f32) {
        let mut clicked_node = None;
        let mut canvas_clicked = false;
        egui::ScrollArea::both()
            .id_salt("exact_control_graph_canvas")
            .auto_shrink([false, false])
            .max_height(maximum_height)
            .show(ui, |ui| {
                let (canvas, painter) =
                    ui.allocate_painter(self.presentation.size, egui::Sense::click());
                canvas_clicked = canvas.clicked();
                painter.rect_filled(canvas.rect, 0.0, egui::Color32::from_rgb(17, 21, 29));
                paint_grid(&painter, canvas.rect);
                let origin = canvas.rect.min.to_vec2();

                for wire in self.fixture.document().wires() {
                    self.paint_wire(&painter, origin, wire.id(), wire.source(), wire.target());
                }
                for node in self.fixture.document().nodes() {
                    let Some(presentation) = self.presentation.nodes.get(&node.id()) else {
                        continue;
                    };
                    let rect = presentation.rect.translate(origin);
                    let response = ui.interact(
                        rect,
                        egui::Id::new(("exact_control_node", node.id().get())),
                        egui::Sense::click(),
                    );
                    if response.clicked() {
                        clicked_node = Some(node.id());
                    }
                    self.paint_node(&painter, rect, node, presentation.rank);
                }
            });
        if let Some(node) = clicked_node {
            self.selected_node = Some(node);
        } else if canvas_clicked {
            self.selected_node = None;
        }
    }

    fn paint_wire(
        &self,
        painter: &egui::Painter,
        origin: egui::Vec2,
        id: GraphWireId,
        source: WireEndpoint,
        target: WireEndpoint,
    ) {
        let (Some(source_anchor), Some(target_anchor)) = (
            port_anchor(self.fixture.document(), &self.presentation, source, true),
            port_anchor(self.fixture.document(), &self.presentation, target, false),
        ) else {
            return;
        };
        let source_anchor = source_anchor + origin;
        let target_anchor = target_anchor + origin;
        let selected = self
            .selected_node
            .is_some_and(|node| node == source.node || node == target.node);
        let color = if selected {
            egui::Color32::WHITE
        } else {
            wire_color(self.fixture.document(), source)
        };
        let stroke = egui::Stroke::new(if selected { 2.4_f32 } else { 1.5_f32 }, color);
        let feedback_lane = self
            .presentation
            .wires
            .get(&id)
            .and_then(|wire| wire.feedback_lane);
        let points = if let Some(lane) = feedback_lane {
            let route_y = origin.y + self.presentation.size.y - 18.0 - display_index(lane) * 13.0;
            vec![
                source_anchor,
                egui::pos2(source_anchor.x + 22.0, source_anchor.y),
                egui::pos2(source_anchor.x + 22.0, route_y),
                egui::pos2(target_anchor.x - 22.0, route_y),
                egui::pos2(target_anchor.x - 22.0, target_anchor.y),
                target_anchor,
            ]
        } else {
            let middle_x = (source_anchor.x + target_anchor.x) * 0.5;
            vec![
                source_anchor,
                egui::pos2(middle_x, source_anchor.y),
                egui::pos2(middle_x, target_anchor.y),
                target_anchor,
            ]
        };
        painter.add(egui::Shape::line(points, stroke));
        painter.add(egui::Shape::convex_polygon(
            vec![
                target_anchor,
                target_anchor + egui::vec2(-7.0, -4.0),
                target_anchor + egui::vec2(-7.0, 4.0),
            ],
            color,
            egui::Stroke::NONE,
        ));
    }

    fn paint_node(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        node: &NodeDefinition,
        rank: usize,
    ) {
        let selected = self.selected_node == Some(node.id());
        let stateful = self
            .fixture
            .registry()
            .semantic_registry()
            .schema(node.kind())
            .is_some_and(|schema| schema.state().is_some());
        let fill = if selected {
            egui::Color32::from_rgb(46, 73, 105)
        } else if stateful {
            egui::Color32::from_rgb(49, 47, 67)
        } else {
            egui::Color32::from_rgb(34, 43, 56)
        };
        let border = if selected {
            egui::Color32::from_rgb(126, 195, 255)
        } else if stateful {
            egui::Color32::from_rgb(209, 158, 255)
        } else {
            egui::Color32::from_rgb(89, 111, 139)
        };
        painter.rect_filled(rect, 6.0, fill);
        painter.rect_stroke(
            rect,
            6.0,
            egui::Stroke::new(if selected { 2.0_f32 } else { 1.0_f32 }, border),
        );
        painter.line_segment(
            [
                egui::pos2(rect.left(), rect.top() + NODE_HEADER_HEIGHT),
                egui::pos2(rect.right(), rect.top() + NODE_HEADER_HEIGHT),
            ],
            egui::Stroke::new(1.0_f32, border.gamma_multiply(0.65)),
        );
        painter.text(
            rect.left_top() + egui::vec2(10.0, 9.0),
            egui::Align2::LEFT_TOP,
            node.label(),
            egui::FontId::proportional(13.0),
            egui::Color32::WHITE,
        );
        painter.text(
            rect.left_top() + egui::vec2(10.0, 28.0),
            egui::Align2::LEFT_TOP,
            format!(
                "L{rank} · {} · v{}",
                short_kind(node.kind().name()),
                node.kind().version()
            ),
            egui::FontId::monospace(10.5),
            egui::Color32::GRAY,
        );

        for (index, port) in node.inputs().iter().enumerate() {
            let anchor = port_anchor_for_rect(rect, index, false);
            painter.circle_filled(anchor, 4.0, egui::Color32::from_rgb(136, 171, 211));
            painter.text(
                anchor + egui::vec2(9.0, 0.0),
                egui::Align2::LEFT_CENTER,
                port.name(),
                egui::FontId::proportional(11.0),
                egui::Color32::LIGHT_GRAY,
            );
        }
        for (index, port) in node.outputs().iter().enumerate() {
            let anchor = port_anchor_for_rect(rect, index, true);
            painter.circle_filled(anchor, 4.0, egui::Color32::from_rgb(122, 211, 185));
            painter.text(
                anchor + egui::vec2(-9.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                port.name(),
                egui::FontId::proportional(11.0),
                egui::Color32::LIGHT_GRAY,
            );
        }
        let footer = match (stateful, node.parameters().len()) {
            (true, count) => format!("unit delay · {count} exact parameter(s)"),
            (false, 0) => "HostExact".to_owned(),
            (false, count) => format!("HostExact · {count} exact parameter(s)"),
        };
        painter.text(
            rect.left_bottom() + egui::vec2(10.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            footer,
            egui::FontId::monospace(9.5),
            egui::Color32::from_rgb(170, 180, 194),
        );
    }

    fn show_selected_node(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected_node else {
            ui.weak("Select a node to inspect exact ports, parameters, and state authority.");
            return;
        };
        let Some(node) = self.fixture.document().node(id) else {
            return;
        };
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong(format!("#{} {}", id.get(), node.label()));
                ui.monospace(format!("{} v{}", node.kind().name(), node.kind().version()));
                if ui.small_button("clear").clicked() {
                    self.selected_node = None;
                }
            });
            ui.label(format!("Execution domain: {}", domain_label(node.domain())));
            ui.horizontal_wrapped(|ui| {
                for port in node.inputs() {
                    ui.monospace(port_description(self.fixture.document(), "in", port));
                }
                for port in node.outputs() {
                    ui.monospace(port_description(self.fixture.document(), "out", port));
                }
            });
            for parameter in node.parameters() {
                ui.monospace(format!(
                    "{} = {}",
                    parameter.name(),
                    typed_value_text(self.fixture.document(), parameter.value())
                ));
            }
            if let Some(state) = self
                .fixture
                .registry()
                .semantic_registry()
                .schema(node.kind())
                .and_then(alumina_interface_core::graph::NodeSchema::state)
            {
                ui.label(format!(
                    "Explicit state: clock {}, t{}, read-before-write, ≤{} canonical bytes",
                    state.clock().get(),
                    state.value_type().get(),
                    state.declared_storage_bytes()
                ));
            }
        });
    }

    fn show_trace(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Exact control trace");
            ui.label("10 Hz clock · six retained ticks · certified f64 display enclosures");
        });
        let width = ui.available_width().max(120.0);
        let (response, painter) =
            ui.allocate_painter(egui::vec2(width, 214.0), egui::Sense::hover());
        let plot = egui::Rect::from_min_max(
            response.rect.min + egui::vec2(42.0, 14.0),
            response.rect.max - egui::vec2(12.0, 31.0),
        );
        painter.rect_filled(plot, 3.0, egui::Color32::from_rgb(17, 21, 29));

        let maximum_tick = self
            .traces
            .iter()
            .flat_map(|series| series.points.iter().map(|point| point.tick))
            .max()
            .unwrap_or(1)
            .max(1);
        let (minimum_value, maximum_value) = trace_value_bounds(&self.traces);
        paint_trace_grid(&painter, plot, maximum_tick, minimum_value, maximum_value);

        if let Some(pointer) = response
            .hover_pos()
            .filter(|position| plot.contains(*position))
        {
            self.cursor_tick = cursor_tick(plot, pointer.x, maximum_tick);
        }
        let cursor_x = plot_x(plot, self.cursor_tick, maximum_tick);
        painter.line_segment(
            [
                egui::pos2(cursor_x, plot.top()),
                egui::pos2(cursor_x, plot.bottom()),
            ],
            egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(110)),
        );

        for series in &self.traces {
            let color = signal_color(series.signal);
            let mut line: Vec<egui::Pos2> = Vec::new();
            for point in &series.points {
                let x = plot_x(plot, point.tick, maximum_tick);
                let lower = plot_y(plot, point.enclosure[0], minimum_value, maximum_value);
                let upper = plot_y(plot, point.enclosure[1], minimum_value, maximum_value);
                let middle = (lower + upper) * 0.5;
                if let Some(previous) = line.last().copied() {
                    line.push(egui::pos2(x, previous.y));
                }
                line.push(egui::pos2(x, middle));
                painter.line_segment(
                    [egui::pos2(x, lower), egui::pos2(x, upper)],
                    egui::Stroke::new(3.0_f32, color.gamma_multiply(0.55)),
                );
                painter.circle_filled(egui::pos2(x, middle), 2.8, color);
            }
            painter.add(egui::Shape::line(line, egui::Stroke::new(1.8_f32, color)));
        }

        painter.text(
            egui::pos2(plot.left(), response.rect.bottom() - 8.0),
            egui::Align2::LEFT_BOTTOM,
            "control clock tick",
            egui::FontId::proportional(10.0),
            egui::Color32::GRAY,
        );
        painter.text(
            egui::pos2(response.rect.left() + 6.0, plot.top()),
            egui::Align2::LEFT_TOP,
            "mm",
            egui::FontId::proportional(10.0),
            egui::Color32::GRAY,
        );

        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("tick {}", self.cursor_tick));
            for series in &self.traces {
                if let Some(point) = series
                    .points
                    .iter()
                    .find(|point| point.tick == self.cursor_tick)
                {
                    ui.colored_label(
                        signal_color(series.signal),
                        format!("{} = {} mm", series.signal.label(), point.exact),
                    );
                }
            }
        });
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "layout admission, state-edge classification, ranking, and bounded placement stay together for auditability"
)]
fn graph_presentation(
    document: &GraphDocument,
    registry: &GraphSimulationRegistry,
) -> Result<GraphPresentation, String> {
    if document.nodes().is_empty() {
        return Err("exact control graph has no nodes".to_owned());
    }
    if document.nodes().len() > MAXIMUM_VISIBLE_NODES {
        return Err("exact control graph exceeds the visible-node limit".to_owned());
    }
    if document.wires().len() > MAXIMUM_VISIBLE_WIRES {
        return Err("exact control graph exceeds the visible-wire limit".to_owned());
    }

    let mut instantaneous_edges = BTreeSet::new();
    let mut feedback = BTreeSet::new();
    for wire in document.wires() {
        let target = document
            .node(wire.target().node)
            .ok_or_else(|| format!("wire {} has no target node", wire.id().get()))?;
        let schema = registry
            .semantic_registry()
            .schema(target.kind())
            .ok_or_else(|| format!("node {} has no audited schema", target.id().get()))?;
        let is_state_capture = schema
            .state()
            .is_some_and(|state| state.next_input() == wire.target().port);
        let has_current_tick_effect = schema.outputs().is_empty()
            || schema
                .output_dependencies()
                .iter()
                .any(|dependency| dependency.inputs().contains(&wire.target().port));
        if is_state_capture || !has_current_tick_effect {
            feedback.insert(wire.id());
        } else {
            instantaneous_edges.insert((wire.source().node, wire.target().node));
        }
    }

    let mut indegree: BTreeMap<GraphNodeId, usize> =
        document.nodes().iter().map(|node| (node.id(), 0)).collect();
    let mut successors: BTreeMap<GraphNodeId, Vec<GraphNodeId>> = BTreeMap::new();
    for (source, target) in &instantaneous_edges {
        let Some(value) = indegree.get_mut(target) else {
            return Err(format!("layout target node {} is missing", target.get()));
        };
        *value = value
            .checked_add(1)
            .ok_or_else(|| "graph layout indegree overflow".to_owned())?;
        successors.entry(*source).or_default().push(*target);
    }

    let mut ranks: BTreeMap<GraphNodeId, usize> =
        document.nodes().iter().map(|node| (node.id(), 0)).collect();
    let mut ready: BTreeSet<GraphNodeId> = indegree
        .iter()
        .filter_map(|(node, degree)| (*degree == 0).then_some(*node))
        .collect();
    let mut visited = 0_usize;
    while let Some(node) = ready.pop_first() {
        visited += 1;
        let source_rank = *ranks
            .get(&node)
            .ok_or_else(|| "graph layout rank is missing".to_owned())?;
        for target in successors.get(&node).into_iter().flatten() {
            let candidate = source_rank
                .checked_add(1)
                .ok_or_else(|| "graph layout rank overflow".to_owned())?;
            let target_rank = ranks
                .get_mut(target)
                .ok_or_else(|| "graph layout target rank is missing".to_owned())?;
            *target_rank = (*target_rank).max(candidate);
            let degree = indegree
                .get_mut(target)
                .ok_or_else(|| "graph layout target indegree is missing".to_owned())?;
            *degree = degree
                .checked_sub(1)
                .ok_or_else(|| "graph layout indegree underflow".to_owned())?;
            if *degree == 0 {
                ready.insert(*target);
            }
        }
    }
    if visited != document.nodes().len() {
        return Err("instantaneous graph layout contains a cycle".to_owned());
    }

    let mut columns: BTreeMap<usize, Vec<GraphNodeId>> = BTreeMap::new();
    for node in document.nodes() {
        columns
            .entry(ranks[&node.id()])
            .or_default()
            .push(node.id());
    }
    let mut nodes = BTreeMap::new();
    let mut maximum_bottom = 0.0_f32;
    for (rank, column) in &columns {
        let x = CANVAS_MARGIN + display_index(*rank) * (NODE_WIDTH + COLUMN_GAP);
        let mut y = CANVAS_MARGIN;
        for id in column {
            let node = document
                .node(*id)
                .ok_or_else(|| format!("layout node {} is missing", id.get()))?;
            let height = node_height(node);
            let rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(NODE_WIDTH, height));
            maximum_bottom = maximum_bottom.max(rect.bottom());
            nodes.insert(*id, NodePresentation { rect, rank: *rank });
            y += height + NODE_GAP;
        }
    }
    let maximum_rank = columns.keys().next_back().copied().unwrap_or(0);
    let feedback_height = 36.0 + display_index(feedback.len()) * 13.0;
    let size = egui::vec2(
        CANVAS_MARGIN * 2.0 + NODE_WIDTH + display_index(maximum_rank) * (NODE_WIDTH + COLUMN_GAP),
        maximum_bottom + CANVAS_MARGIN + feedback_height,
    );
    let wires = document
        .wires()
        .iter()
        .map(|wire| {
            let feedback_lane = feedback.iter().position(|id| *id == wire.id());
            (wire.id(), WirePresentation { feedback_lane })
        })
        .collect();
    Ok(GraphPresentation { nodes, wires, size })
}

fn trace_series(fixture: &RepresentativeExactControlGraph) -> Result<Vec<TraceSeries>, String> {
    let mut result = Vec::with_capacity(SIGNALS.len());
    for signal in SIGNALS {
        let mut points = Vec::new();
        for entry in fixture.simulation().entries().iter().filter(|entry| {
            entry.kind() == GraphTraceEntryKind::NodeOutput && entry.endpoint() == signal.endpoint()
        }) {
            if points.len() >= MAXIMUM_POINTS_PER_SERIES {
                return Err(format!("{} trace exceeds display policy", signal.label()));
            }
            let GraphValue::ExactRational(value) = entry.value().value() else {
                return Err(format!("{} trace is not exact rational", signal.label()));
            };
            let enclosure = value
                .to_f64_enclosure()
                .filter(|bounds| bounds.iter().all(|bound| bound.is_finite()))
                .ok_or_else(|| format!("{} has no finite display enclosure", signal.label()))?;
            points.push(TracePoint {
                tick: entry.clock_tick(),
                exact: value.to_string(),
                enclosure,
            });
        }
        if points.is_empty() {
            return Err(format!("{} trace is empty", signal.label()));
        }
        result.push(TraceSeries { signal, points });
    }
    Ok(result)
}

fn node_height(node: &NodeDefinition) -> f32 {
    let port_rows = node.inputs().len().max(node.outputs().len()).max(1);
    NODE_HEADER_HEIGHT + display_index(port_rows) * PORT_ROW_HEIGHT + 30.0
}

fn port_anchor(
    document: &GraphDocument,
    presentation: &GraphPresentation,
    endpoint: WireEndpoint,
    output: bool,
) -> Option<egui::Pos2> {
    let node = document.node(endpoint.node)?;
    let layout = presentation.nodes.get(&endpoint.node)?;
    let ports = if output {
        node.outputs()
    } else {
        node.inputs()
    };
    let index = ports.iter().position(|port| port.id() == endpoint.port)?;
    Some(port_anchor_for_rect(layout.rect, index, output))
}

fn port_anchor_for_rect(rect: egui::Rect, index: usize, output: bool) -> egui::Pos2 {
    let x = if output { rect.right() } else { rect.left() };
    egui::pos2(
        x,
        rect.top() + NODE_HEADER_HEIGHT + PORT_ROW_HEIGHT * (display_index(index) + 0.5),
    )
}

fn wire_color(document: &GraphDocument, source: WireEndpoint) -> egui::Color32 {
    let boolean = document
        .node(source.node)
        .and_then(|node| node.outputs().iter().find(|port| port.id() == source.port))
        .and_then(|port| document.schema().value_type(port.value_type()))
        .and_then(|value_type| match value_type.kind() {
            TypeKind::Stream { sample, .. } => document.schema().value_type(*sample),
            _ => None,
        })
        .is_some_and(|sample| matches!(sample.kind(), TypeKind::Boolean));
    if boolean {
        egui::Color32::from_rgb(241, 178, 84)
    } else {
        egui::Color32::from_rgb(96, 169, 232)
    }
}

fn typed_value_text(
    document: &GraphDocument,
    value: &alumina_interface_core::graph::TypedGraphValue,
) -> String {
    match value.value() {
        GraphValue::ExactRational(exact) => {
            let symbol = document
                .schema()
                .value_type(value.value_type())
                .and_then(|value_type| match value_type.kind() {
                    TypeKind::ExactRational { unit } => document.schema().unit(*unit),
                    _ => None,
                })
                .map_or("", alumina_interface_core::graph::UnitDefinition::symbol);
            format!("{exact} {symbol}")
        }
        GraphValue::Boolean(value) => value.to_string(),
        other => format!("{other:?}"),
    }
}

fn port_description(
    document: &GraphDocument,
    direction: &str,
    port: &alumina_interface_core::graph::PortDefinition,
) -> String {
    let type_name = document.schema().value_type(port.value_type()).map_or(
        "unknown",
        alumina_interface_core::graph::TypeDefinition::name,
    );
    format!(
        "{direction}.{}: {type_name} [t{}]",
        port.name(),
        port.value_type().get()
    )
}

fn paint_grid(painter: &egui::Painter, rect: egui::Rect) {
    let stroke = egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(13));
    let spacing = 24.0;
    let mut x = rect.left();
    while x <= rect.right() {
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            stroke,
        );
        x += spacing;
    }
    let mut y = rect.top();
    while y <= rect.bottom() {
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            stroke,
        );
        y += spacing;
    }
}

fn paint_trace_grid(
    painter: &egui::Painter,
    rect: egui::Rect,
    maximum_tick: u64,
    minimum_value: f64,
    maximum_value: f64,
) {
    let grid = egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(28));
    for tick in 0..=maximum_tick {
        let x = plot_x(rect, tick, maximum_tick);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            grid,
        );
        painter.text(
            egui::pos2(x, rect.bottom() + 4.0),
            egui::Align2::CENTER_TOP,
            tick.to_string(),
            egui::FontId::monospace(9.5),
            egui::Color32::GRAY,
        );
    }
    for index in 0..=4 {
        let fraction = f64::from(index) / 4.0;
        let value = minimum_value + (maximum_value - minimum_value) * fraction;
        let y = plot_y(rect, value, minimum_value, maximum_value);
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            grid,
        );
        painter.text(
            egui::pos2(rect.left() - 5.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{value:.1}"),
            egui::FontId::monospace(9.0),
            egui::Color32::GRAY,
        );
    }
}

fn trace_value_bounds(series: &[TraceSeries]) -> (f64, f64) {
    let mut minimum = 0.0_f64;
    let mut maximum = 0.0_f64;
    for point in series.iter().flat_map(|series| &series.points) {
        minimum = minimum.min(point.enclosure[0]);
        maximum = maximum.max(point.enclosure[1]);
    }
    let span = (maximum - minimum).max(1.0);
    (minimum - span * 0.1, maximum + span * 0.1)
}

#[allow(
    clippy::cast_precision_loss,
    reason = "bounded indices are projected only into non-authoritative egui coordinates"
)]
fn display_index(value: usize) -> f32 {
    value as f32
}

#[allow(
    clippy::cast_precision_loss,
    reason = "bounded trace ticks are projected only into non-authoritative egui coordinates"
)]
fn plot_x(rect: egui::Rect, tick: u64, maximum_tick: u64) -> f32 {
    rect.left() + rect.width() * (tick as f32 / maximum_tick as f32)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "certified finite enclosure coordinates are intentionally projected to egui f32"
)]
fn plot_y(rect: egui::Rect, value: f64, minimum: f64, maximum: f64) -> f32 {
    let fraction = ((value - minimum) / (maximum - minimum)).clamp(0.0, 1.0) as f32;
    rect.bottom() - rect.height() * fraction
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a clamped hover coordinate selects one bounded display tick"
)]
fn cursor_tick(rect: egui::Rect, x: f32, maximum_tick: u64) -> u64 {
    let fraction = ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    (fraction * maximum_tick as f32).round() as u64
}

const fn signal_color(signal: RepresentativeControlSignal) -> egui::Color32 {
    match signal {
        RepresentativeControlSignal::Error => egui::Color32::from_rgb(247, 196, 86),
        RepresentativeControlSignal::IntegralPrior => egui::Color32::from_rgb(91, 205, 224),
        RepresentativeControlSignal::ClampedController => egui::Color32::from_rgb(102, 221, 142),
        RepresentativeControlSignal::PermittedOutput => egui::Color32::from_rgb(245, 121, 169),
    }
}

fn short_kind(kind: &str) -> &str {
    kind.strip_prefix("control.").unwrap_or(kind)
}

const fn domain_label(domain: ExecutionDomain) -> &'static str {
    match domain {
        ExecutionDomain::HostExact => "HostExact",
        ExecutionDomain::Service { .. } => "Service",
        ExecutionDomain::Realtime { .. } => "Realtime",
    }
}

fn digest_prefix(digest: [u8; 32]) -> String {
    use std::fmt::Write as _;

    let mut result = String::with_capacity(16);
    for byte in &digest[..8] {
        write!(&mut result, "{byte:02x}").expect("writing to String is infallible");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn representative_layout_is_bounded_acyclic_and_keeps_feedback_visible() {
        let workspace = ExactControlWorkspace::try_new().unwrap();
        assert_eq!(workspace.presentation.nodes.len(), 19);
        assert_eq!(workspace.presentation.wires.len(), 22);
        assert_eq!(
            workspace
                .presentation
                .wires
                .values()
                .filter(|wire| wire.feedback_lane.is_some())
                .count(),
            2
        );
        let nodes: Vec<_> = workspace.presentation.nodes.values().collect();
        for (index, left) in nodes.iter().enumerate() {
            for right in &nodes[index + 1..] {
                assert!(!left.rect.intersects(right.rect));
            }
        }
        assert!(nodes.iter().any(|node| node.rank == 0));
        assert_eq!(workspace.traces.len(), SIGNALS.len());
        assert!(
            workspace
                .traces
                .iter()
                .all(|series| series.points.len() == 6)
        );
    }

    #[test]
    fn headless_exact_control_workspace_produces_a_complete_egui_frame() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let context = egui::Context::default();
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1_280.0, 900.0),
                )),
                ..egui::RawInput::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| workspace.show(ui));
            },
        );
        assert!(!output.shapes.is_empty());
        assert!(!output.textures_delta.set.is_empty());
    }
}
