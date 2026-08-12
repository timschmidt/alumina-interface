//! Bounded browser/native presentation of the representative exact controller.
//!
//! All graph and trace authority remains in `alumina-interface-core`. This
//! module performs only deterministic layout and named, lossy display
//! projection into egui coordinates.

use std::collections::{BTreeMap, BTreeSet};

use alumina_interface_core::graph::{
    CanonicalGraphWorkspaceEncoding, ExecutionDomain, GraphDocument, GraphNodeId,
    GraphNodePlacement, GraphSimulationRegistry, GraphTraceEntryKind, GraphValue, GraphWireId,
    GraphWorkspaceDocument, GraphWorkspaceLimits, NodeDefinition, RepresentativeControlSignal,
    RepresentativeExactControlGraph, TypeKind, WireEndpoint, analyze_graph,
    compile_representative_exact_control_graph, encode_graph_workspace,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NodeDrag {
    node: GraphNodeId,
    origin: GraphNodePlacement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PortEdit {
    SelectOutput(WireEndpoint),
    ConnectInput(WireEndpoint),
    DisconnectInput(WireEndpoint),
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
    workspace: GraphWorkspaceDocument,
    workspace_encoding: CanonicalGraphWorkspaceEncoding,
    presentation: GraphPresentation,
    traces: Vec<TraceSeries>,
    selected_node: Option<GraphNodeId>,
    pending_source: Option<WireEndpoint>,
    drag: Option<NodeDrag>,
    edit_status: String,
    cursor_tick: u64,
}

impl ExactControlWorkspace {
    pub(crate) fn try_new() -> Result<Self, String> {
        let fixture =
            compile_representative_exact_control_graph().map_err(|error| error.to_string())?;
        let (workspace, workspace_encoding, presentation) = initial_workspace(&fixture)?;
        let traces = trace_series(&fixture)?;
        Ok(Self {
            fixture,
            workspace,
            workspace_encoding,
            presentation,
            traces,
            selected_node: None,
            pending_source: None,
            drag: None,
            edit_status: "canonical workspace ready; no structural edits".to_owned(),
            cursor_tick: 0,
        })
    }

    pub(crate) fn show_sidebar(&self, ui: &mut egui::Ui) {
        let document = self.workspace.graph();
        ui.label("Exact control graph workspace");
        ui.label(format!(
            "Draft nodes / wires: {} / {} · revision {}",
            document.nodes().len(),
            document.wires().len(),
            self.workspace.revision()
        ));
        ui.label(format!(
            "Canonical workspace: {} bytes",
            self.workspace_encoding.bytes().len()
        ));
        ui.monospace(format!(
            "workspace {}…",
            digest_prefix(self.workspace_encoding.digest().0)
        ));
        ui.label(format!(
            "Reference trace: {} entries / {} bytes",
            self.fixture.simulation().entries().len(),
            self.fixture.trace().bytes().len()
        ));
        ui.monospace(format!(
            "draft graph {}…",
            digest_prefix(self.workspace.graph_digest().0)
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
            "Editor draft only — no deployment, firmware, or output authority.",
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
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.heading("Exact PID / interlock");
            ui.label("50 Hz acquisition to 10 Hz control");
            let trace_current = self.reference_trace_is_current();
            ui.colored_label(
                if trace_current {
                    egui::Color32::LIGHT_BLUE
                } else {
                    egui::Color32::YELLOW
                },
                if trace_current {
                    "canonical reference replay attached"
                } else {
                    "draft topology changed; reference replay detached"
                },
            );
            if ui.small_button("reset draft").clicked() {
                self.reset_draft();
            }
        });
        ui.label(
            "Drag node headers to record integer canvas positions in the in-memory ALGW draft. Click an output then an input to connect; secondary-click an input to disconnect. Editing never arms or commands firmware.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(egui::Color32::from_rgb(96, 169, 232), "— exact Stream");
            ui.colored_label(egui::Color32::from_rgb(241, 178, 84), "— Boolean Stream");
            ui.colored_label(egui::Color32::from_rgb(209, 158, 255), "outlined state");
            if let Some(source) = self.pending_source {
                ui.colored_label(
                    egui::Color32::WHITE,
                    format!("wiring #{}.{}", source.node.get(), source.port.get()),
                );
                if ui.small_button("cancel wire").clicked() {
                    self.pending_source = None;
                    "pending wire cancelled".clone_into(&mut self.edit_status);
                }
            }
        });
        ui.label(&self.edit_status);
        ui.separator();

        let graph_height = (ui.available_height() * 0.5).clamp(230.0, 430.0);
        self.show_graph(ui, graph_height);
        self.show_selected_node(ui);
        ui.separator();
        if self.reference_trace_is_current() {
            self.show_trace(ui);
        } else {
            ui.colored_label(
                egui::Color32::YELLOW,
                "The exact trace is hidden because ALGT binds the unedited reference graph. Reset the draft or simulate a newly reviewed graph before plotting it.",
            );
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "canvas allocation, layered node/port interaction, preview, and deferred transactional edits remain one egui frame operation"
    )]
    fn show_graph(&mut self, ui: &mut egui::Ui, maximum_height: f32) {
        let nodes = self.workspace.graph().nodes().to_vec();
        let wires = self.workspace.graph().wires().to_vec();
        let mut clicked_node = None;
        let mut canvas_clicked = false;
        let mut move_request = None;
        let mut port_edit = None;
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

                for wire in &wires {
                    self.paint_wire(&painter, origin, wire.id(), wire.source(), wire.target());
                }
                for node in &nodes {
                    let Some(presentation) = self.presentation.nodes.get(&node.id()) else {
                        continue;
                    };
                    let rect = presentation.rect.translate(origin);
                    let header_rect = egui::Rect::from_min_max(
                        rect.min,
                        egui::pos2(rect.right(), rect.top() + NODE_HEADER_HEIGHT),
                    );
                    let response = ui.interact(
                        header_rect,
                        egui::Id::new(("exact_control_node", node.id().get())),
                        egui::Sense::click_and_drag(),
                    );
                    if response.drag_started()
                        && let Some(placement) = self.workspace.placement(node.id())
                    {
                        self.drag = Some(NodeDrag {
                            node: node.id(),
                            origin: placement,
                        });
                    }
                    let dragging = self
                        .drag
                        .filter(|drag| drag.node == node.id())
                        .filter(|_| response.dragged() || response.drag_stopped());
                    let painted_rect =
                        dragging.map_or(rect, |_| rect.translate(response.drag_delta()));
                    if response.drag_stopped()
                        && let Some(drag) = dragging
                    {
                        move_request = Some((drag, response.drag_delta()));
                        self.drag = None;
                    }
                    if response.clicked() {
                        clicked_node = Some(node.id());
                    }
                    for (index, port) in node.inputs().iter().enumerate() {
                        let anchor = port_anchor_for_rect(painted_rect, index, false);
                        let port_response = ui.interact(
                            egui::Rect::from_center_size(anchor, egui::vec2(18.0, 18.0)),
                            egui::Id::new((
                                "exact_control_input",
                                node.id().get(),
                                port.id().get(),
                            )),
                            egui::Sense::click(),
                        );
                        let endpoint = WireEndpoint {
                            node: node.id(),
                            port: port.id(),
                        };
                        if port_response.secondary_clicked() {
                            port_edit = Some(PortEdit::DisconnectInput(endpoint));
                            clicked_node = Some(node.id());
                        } else if port_response.clicked() {
                            port_edit = Some(PortEdit::ConnectInput(endpoint));
                            clicked_node = Some(node.id());
                        }
                    }
                    for (index, port) in node.outputs().iter().enumerate() {
                        let anchor = port_anchor_for_rect(painted_rect, index, true);
                        let port_response = ui.interact(
                            egui::Rect::from_center_size(anchor, egui::vec2(18.0, 18.0)),
                            egui::Id::new((
                                "exact_control_output",
                                node.id().get(),
                                port.id().get(),
                            )),
                            egui::Sense::click(),
                        );
                        if port_response.clicked() {
                            port_edit = Some(PortEdit::SelectOutput(WireEndpoint {
                                node: node.id(),
                                port: port.id(),
                            }));
                            clicked_node = Some(node.id());
                        }
                    }
                    self.paint_node(&painter, painted_rect, node, presentation.rank);
                }
                if let Some(source) = self.pending_source
                    && let Some(source_anchor) =
                        port_anchor(self.workspace.graph(), &self.presentation, source, true)
                    && let Some(pointer) = ui.ctx().pointer_hover_pos()
                {
                    painter.line_segment(
                        [source_anchor + origin, pointer],
                        egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
                    );
                }
            });
        let interaction_consumed =
            clicked_node.is_some() || move_request.is_some() || port_edit.is_some();
        if let Some((drag, delta)) = move_request {
            self.commit_node_drag(drag, delta);
        }
        if let Some(edit) = port_edit {
            self.handle_port_edit(edit);
        }
        if let Some(node) = clicked_node {
            self.selected_node = Some(node);
        } else if canvas_clicked && !interaction_consumed {
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
            port_anchor(self.workspace.graph(), &self.presentation, source, true),
            port_anchor(self.workspace.graph(), &self.presentation, target, false),
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
            wire_color(self.workspace.graph(), source)
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

    fn reference_trace_is_current(&self) -> bool {
        self.workspace.graph_digest() == self.fixture.simulation().graph_digest()
    }

    fn reset_draft(&mut self) {
        match initial_workspace(&self.fixture) {
            Ok((workspace, encoding, presentation)) => {
                self.workspace = workspace;
                self.workspace_encoding = encoding;
                self.presentation = presentation;
                self.pending_source = None;
                self.drag = None;
                "draft reset to the canonical reference graph and layout"
                    .clone_into(&mut self.edit_status);
            }
            Err(error) => {
                self.edit_status = format!("draft reset failed without mutation: {error}");
            }
        }
    }

    fn commit_node_drag(&mut self, drag: NodeDrag, delta: egui::Vec2) {
        let (x, y) = match (
            quantized_canvas_coordinate(drag.origin.x(), delta.x),
            quantized_canvas_coordinate(drag.origin.y(), delta.y),
        ) {
            (Ok(x), Ok(y)) => (x, y),
            (Err(error), _) | (_, Err(error)) => {
                self.edit_status = format!("node move rejected without mutation: {error}");
                return;
            }
        };
        let mut candidate = self.workspace.clone();
        if let Err(error) = candidate.move_node(drag.node, x, y) {
            self.edit_status = format!("node move rejected without mutation: {error}");
            return;
        }
        self.commit_candidate(
            candidate,
            &format!(
                "moved node {} to canonical canvas ({x}, {y})",
                drag.node.get()
            ),
        );
    }

    fn handle_port_edit(&mut self, edit: PortEdit) {
        match edit {
            PortEdit::SelectOutput(source) => {
                if self.pending_source == Some(source) {
                    self.pending_source = None;
                    "pending wire cancelled".clone_into(&mut self.edit_status);
                } else {
                    self.pending_source = Some(source);
                    self.edit_status = format!(
                        "selected output #{}.{}; choose one typed input",
                        source.node.get(),
                        source.port.get()
                    );
                }
            }
            PortEdit::ConnectInput(target) => {
                let Some(source) = self.pending_source else {
                    self.edit_status = format!(
                        "input #{}.{} selected; choose an output first",
                        target.node.get(),
                        target.port.get()
                    );
                    return;
                };
                let mut candidate = self.workspace.clone();
                let id = match candidate.connect(source, target) {
                    Ok(id) => id,
                    Err(error) => {
                        self.edit_status = format!("wire edit rejected without mutation: {error}");
                        return;
                    }
                };
                if self.commit_candidate(
                    candidate,
                    &format!(
                        "connected wire {} from #{}.{} to #{}.{}",
                        id.get(),
                        source.node.get(),
                        source.port.get(),
                        target.node.get(),
                        target.port.get()
                    ),
                ) {
                    self.pending_source = None;
                }
            }
            PortEdit::DisconnectInput(target) => {
                let Some(id) = self
                    .workspace
                    .graph()
                    .wires()
                    .iter()
                    .find(|wire| wire.target() == target)
                    .map(|wire| wire.id())
                else {
                    self.edit_status = format!(
                        "input #{}.{} is already disconnected",
                        target.node.get(),
                        target.port.get()
                    );
                    return;
                };
                let mut candidate = self.workspace.clone();
                if let Err(error) = candidate.disconnect(id) {
                    self.edit_status = format!("wire removal rejected without mutation: {error}");
                    return;
                }
                if self.commit_candidate(
                    candidate,
                    &format!(
                        "disconnected wire {} from input #{}.{}",
                        id.get(),
                        target.node.get(),
                        target.port.get()
                    ),
                ) {
                    self.pending_source = None;
                }
            }
        }
    }

    fn commit_candidate(&mut self, candidate: GraphWorkspaceDocument, success: &str) -> bool {
        let presentation = match graph_presentation(
            candidate.graph(),
            self.fixture.registry(),
            Some(candidate.placements()),
        ) {
            Ok(presentation) => presentation,
            Err(error) => {
                self.edit_status = format!("edit rejected without mutation: {error}");
                return false;
            }
        };
        let encoding = match encode_graph_workspace(&candidate) {
            Ok(encoding) => encoding,
            Err(error) => {
                self.edit_status = format!("edit encoding rejected without mutation: {error}");
                return false;
            }
        };
        let semantic = match analyze_graph(
            candidate.graph(),
            self.fixture.registry().semantic_registry(),
        ) {
            Ok(_) => "audited semantics valid".to_owned(),
            Err(error) => format!("draft semantic blocker: {error}"),
        };
        self.workspace = candidate;
        self.workspace_encoding = encoding;
        self.presentation = presentation;
        self.edit_status = format!("{success}; {semantic}");
        true
    }

    fn show_selected_node(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected_node else {
            ui.weak("Select a node to inspect exact ports, parameters, and state authority.");
            return;
        };
        let Some(node) = self.workspace.graph().node(id).cloned() else {
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
            if let Some(placement) = self.workspace.placement(id) {
                ui.monospace(format!(
                    "canvas = ({}, {}) logical px · presentation only",
                    placement.x(),
                    placement.y()
                ));
            }
            ui.horizontal_wrapped(|ui| {
                for port in node.inputs() {
                    ui.monospace(port_description(self.workspace.graph(), "in", port));
                }
                for port in node.outputs() {
                    ui.monospace(port_description(self.workspace.graph(), "out", port));
                }
            });
            for parameter in node.parameters() {
                ui.monospace(format!(
                    "{} = {}",
                    parameter.name(),
                    typed_value_text(self.workspace.graph(), parameter.value())
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

fn initial_workspace(
    fixture: &RepresentativeExactControlGraph,
) -> Result<
    (
        GraphWorkspaceDocument,
        CanonicalGraphWorkspaceEncoding,
        GraphPresentation,
    ),
    String,
> {
    let automatic = graph_presentation(fixture.document(), fixture.registry(), None)?;
    let placements = fixture
        .document()
        .nodes()
        .iter()
        .map(|node| {
            let layout = automatic
                .nodes
                .get(&node.id())
                .ok_or_else(|| format!("automatic layout omitted node {}", node.id().get()))?;
            Ok(GraphNodePlacement::new(
                node.id(),
                canonical_initial_coordinate(layout.rect.left())?,
                canonical_initial_coordinate(layout.rect.top())?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let next_node_id = fixture
        .document()
        .nodes()
        .iter()
        .map(|node| u64::from(node.id().get()))
        .max()
        .unwrap_or(0)
        + 1;
    let next_wire_id = fixture
        .document()
        .wires()
        .iter()
        .map(|wire| u64::from(wire.id().get()))
        .max()
        .unwrap_or(0)
        + 1;
    let workspace = GraphWorkspaceDocument::try_new(
        GraphWorkspaceLimits::interactive(),
        1,
        next_node_id,
        next_wire_id,
        fixture.document().clone(),
        placements,
    )
    .map_err(|error| error.to_string())?;
    let encoding = encode_graph_workspace(&workspace).map_err(|error| error.to_string())?;
    let presentation = graph_presentation(
        workspace.graph(),
        fixture.registry(),
        Some(workspace.placements()),
    )?;
    Ok((workspace, encoding, presentation))
}

#[allow(
    clippy::too_many_lines,
    reason = "layout admission, state-edge classification, ranking, and bounded placement stay together for auditability"
)]
fn graph_presentation(
    document: &GraphDocument,
    registry: &GraphSimulationRegistry,
    placements: Option<&[GraphNodePlacement]>,
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
    let mut maximum_right = 0.0_f32;
    if let Some(placements) = placements {
        if placements.len() != document.nodes().len() {
            return Err("saved canvas does not cover every graph node".to_owned());
        }
        let minimum_x = placements
            .iter()
            .map(|placement| display_coordinate(placement.x()))
            .fold(f32::INFINITY, f32::min);
        let minimum_y = placements
            .iter()
            .map(|placement| display_coordinate(placement.y()))
            .fold(f32::INFINITY, f32::min);
        let offset = egui::vec2(
            (CANVAS_MARGIN - minimum_x).max(0.0),
            (CANVAS_MARGIN - minimum_y).max(0.0),
        );
        for placement in placements {
            let node = document
                .node(placement.node())
                .ok_or_else(|| format!("layout node {} is missing", placement.node().get()))?;
            let height = node_height(node);
            let rect = egui::Rect::from_min_size(
                egui::pos2(
                    display_coordinate(placement.x()) + offset.x,
                    display_coordinate(placement.y()) + offset.y,
                ),
                egui::vec2(NODE_WIDTH, height),
            );
            maximum_bottom = maximum_bottom.max(rect.bottom());
            maximum_right = maximum_right.max(rect.right());
            nodes.insert(
                placement.node(),
                NodePresentation {
                    rect,
                    rank: ranks[&placement.node()],
                },
            );
        }
    } else {
        for (rank, column) in &columns {
            let x = CANVAS_MARGIN + display_index(*rank) * (NODE_WIDTH + COLUMN_GAP);
            let mut y = CANVAS_MARGIN;
            for id in column {
                let node = document
                    .node(*id)
                    .ok_or_else(|| format!("layout node {} is missing", id.get()))?;
                let height = node_height(node);
                let rect =
                    egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(NODE_WIDTH, height));
                maximum_bottom = maximum_bottom.max(rect.bottom());
                maximum_right = maximum_right.max(rect.right());
                nodes.insert(*id, NodePresentation { rect, rank: *rank });
                y += height + NODE_GAP;
            }
        }
    }
    let feedback_height = 36.0 + display_index(feedback.len()) * 13.0;
    let size = egui::vec2(
        maximum_right + CANVAS_MARGIN,
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
    reason = "workspace coordinates are bounded to one million and exactly representable in display f32"
)]
fn display_coordinate(value: i32) -> f32 {
    value as f32
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "automatic layout is finite, integral, bounded presentation metadata"
)]
fn canonical_initial_coordinate(value: f32) -> Result<i32, String> {
    let widened = f64::from(value);
    if !value.is_finite()
        || value.fract() != 0.0
        || widened < f64::from(i32::MIN)
        || widened > f64::from(i32::MAX)
    {
        return Err("automatic canvas coordinate is not a canonical i32".to_owned());
    }
    Ok(widened as i32)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "a finite pointer delta is rounded once into presentation-only integer canvas metadata"
)]
fn quantized_canvas_coordinate(origin: i32, delta: f32) -> Result<i32, String> {
    let projected = f64::from(origin) + f64::from(delta);
    if !projected.is_finite() {
        return Err("pointer delta is not finite".to_owned());
    }
    let rounded = projected.round();
    if rounded < f64::from(i32::MIN) || rounded > f64::from(i32::MAX) {
        return Err("pointer delta exceeds the canvas integer lattice".to_owned());
    }
    Ok(rounded as i32)
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
    use alumina_interface_core::graph::{GraphLimits, GraphPortId, replay_graph_workspace};

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
        let replay = replay_graph_workspace(
            workspace.workspace_encoding.bytes(),
            GraphWorkspaceLimits::interactive(),
            GraphLimits::interactive(),
        )
        .unwrap();
        assert_eq!(workspace.workspace_encoding.bytes().len(), 3_396);
        assert_eq!(
            workspace.workspace_encoding.digest().0,
            [
                0xd7, 0xd4, 0xef, 0x9e, 0x27, 0x35, 0x9a, 0x47, 0x4b, 0x59, 0xf4, 0x8c, 0xdb, 0xcb,
                0x60, 0x4b, 0x3d, 0x4d, 0x16, 0xf2, 0xa7, 0x68, 0xa6, 0x5f, 0x12, 0xc9, 0x5d, 0xde,
                0x8a, 0xee, 0x97, 0x99,
            ]
        );
        assert_eq!(replay.document(), &workspace.workspace);
        assert_eq!(replay.encoding(), &workspace.workspace_encoding);
        assert!(workspace.reference_trace_is_current());
    }

    #[test]
    fn moves_and_port_edits_are_transactional_and_detach_bound_trace() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let graph_digest = workspace.workspace.graph_digest();
        let graph_revision = workspace.workspace.graph().revision();
        let origin = workspace.workspace.placement(GraphNodeId::new(1)).unwrap();
        workspace.commit_node_drag(
            NodeDrag {
                node: GraphNodeId::new(1),
                origin,
            },
            egui::vec2(17.4, -9.6),
        );
        assert_eq!(workspace.workspace.graph_digest(), graph_digest);
        assert_eq!(workspace.workspace.graph().revision(), graph_revision);
        assert_eq!(workspace.workspace.revision(), 2);
        assert_eq!(
            workspace.workspace.placement(GraphNodeId::new(1)),
            Some(GraphNodePlacement::new(
                GraphNodeId::new(1),
                origin.x() + 17,
                origin.y() - 10,
            ))
        );
        assert!(workspace.reference_trace_is_current());

        workspace.handle_port_edit(PortEdit::DisconnectInput(WireEndpoint {
            node: GraphNodeId::new(19),
            port: GraphPortId::new(1),
        }));
        assert_eq!(workspace.workspace.graph().wires().len(), 21);
        assert!(!workspace.reference_trace_is_current());
        assert!(workspace.edit_status.contains("draft semantic blocker"));

        workspace.reset_draft();
        let retained = workspace.workspace.clone();
        workspace.handle_port_edit(PortEdit::SelectOutput(WireEndpoint {
            node: GraphNodeId::new(6),
            port: GraphPortId::new(2),
        }));
        workspace.handle_port_edit(PortEdit::ConnectInput(WireEndpoint {
            node: GraphNodeId::new(19),
            port: GraphPortId::new(1),
        }));
        assert_eq!(workspace.workspace, retained);
        assert!(workspace.edit_status.contains("rejected without mutation"));
        assert_eq!(
            workspace.pending_source,
            Some(WireEndpoint {
                node: GraphNodeId::new(6),
                port: GraphPortId::new(2),
            })
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
