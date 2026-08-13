//! Bounded browser/native presentation of the representative exact controller.
//!
//! All graph and trace authority remains in `alumina-interface-core`. This
//! module performs only deterministic layout and named, lossy display
//! projection into egui coordinates.

use std::collections::{BTreeMap, BTreeSet};

use alumina_interface_core::graph::{
    CanonicalGraphComponentEncoding, CanonicalGraphWorkspaceEncoding, ExecutionDomain,
    GraphComponentDocument, GraphComponentLimits, GraphComponentOutput, GraphComponentOutputId,
    GraphDocument, GraphFrontPanelBinding, GraphFrontPanelItem, GraphFrontPanelItemId,
    GraphFrontPanelRect, GraphLimits, GraphNodeId, GraphNodePlacement, GraphNodePrototype,
    GraphSimulationRegistry, GraphTraceEntryKind, GraphTypeId, GraphValue, GraphWireId,
    GraphWorkspaceDocument, GraphWorkspaceHistory, GraphWorkspaceLimits, NodeDefinition,
    NodeParameter, RepresentativeControlSignal, RepresentativeExactControlGraph, TypeKind,
    TypedGraphValue, WireEndpoint, analyze_graph_draft, compile_representative_exact_control_graph,
    encode_graph_component, encode_graph_workspace, replay_graph_workspace,
};
use eframe::egui;
use hyperreal::Rational;

use crate::workspace_file::{WorkspaceFileBridge, WorkspaceFileEvent};

const MAXIMUM_VISIBLE_NODES: usize = 256;
const MAXIMUM_VISIBLE_WIRES: usize = 1_024;
const MAXIMUM_POINTS_PER_SERIES: usize = 4_096;
const NODE_WIDTH: f32 = 218.0;
const COLUMN_GAP: f32 = 82.0;
const NODE_GAP: f32 = 24.0;
const CANVAS_MARGIN: f32 = 28.0;
const NODE_HEADER_HEIGHT: f32 = 48.0;
const PORT_ROW_HEIGHT: f32 = 20.0;
const NEW_NODE_X_GAP: i32 = 300;
const NEW_NODE_ORIGIN: i32 = 28;
const EMPTY_CANVAS_WIDTH: f32 = 720.0;
const EMPTY_CANVAS_HEIGHT: f32 = 280.0;
const PERSISTED_WORKSPACE_PREFIX: &str = "algw1:";
const MAXIMUM_PERSISTED_WORKSPACE_BYTES: usize = 2 * 1024 * 1024;
#[cfg(target_arch = "wasm32")]
pub(crate) const WORKSPACE_STORAGE_KEY: &str = "alumina.graph-workspace.algw.v1";
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

#[derive(Clone, Debug)]
struct NodePaletteEntry {
    display_name: String,
    prototype: GraphNodePrototype,
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

#[derive(Clone, Debug)]
struct ComponentPackage {
    document: GraphComponentDocument,
    encoding: CanonicalGraphComponentEncoding,
}

#[derive(Clone, Debug)]
struct ComponentPanelItem {
    name: String,
    binding: GraphFrontPanelBinding,
    rect: GraphFrontPanelRect,
    value_type: GraphTypeId,
    output_text: Option<String>,
}

/// Browser/native inspector for one shared exact control graph and trace.
pub(crate) struct ExactControlWorkspace {
    fixture: RepresentativeExactControlGraph,
    workspace: GraphWorkspaceDocument,
    workspace_encoding: CanonicalGraphWorkspaceEncoding,
    component: Option<ComponentPackage>,
    component_status: String,
    history: GraphWorkspaceHistory,
    presentation: GraphPresentation,
    traces: Vec<TraceSeries>,
    palette: Vec<NodePaletteEntry>,
    palette_index: usize,
    parameter_drafts: BTreeMap<(GraphNodeId, u32), String>,
    selected_node: Option<GraphNodeId>,
    pending_source: Option<WireEndpoint>,
    drag: Option<NodeDrag>,
    edit_status: String,
    persistence_dirty: bool,
    persistence_attempted: bool,
    file_status: String,
    file_bridge: WorkspaceFileBridge,
    cursor_tick: u64,
}

impl ExactControlWorkspace {
    #[cfg(test)]
    pub(crate) fn try_new() -> Result<Self, String> {
        Self::try_new_with_persisted(None)
    }

    pub(crate) fn try_new_with_persisted(persisted: Option<&str>) -> Result<Self, String> {
        let fixture =
            compile_representative_exact_control_graph().map_err(|error| error.to_string())?;
        let (workspace, workspace_encoding, presentation) = initial_workspace(&fixture)?;
        let component = representative_component(&workspace)?;
        let traces = trace_series(&fixture)?;
        let palette = control_palette(&fixture)?;
        let mut result = Self {
            fixture,
            workspace,
            workspace_encoding,
            component: Some(component),
            component_status: "canonical ALGC connector pane and front panel attached".to_owned(),
            history: GraphWorkspaceHistory::default(),
            presentation,
            traces,
            palette,
            palette_index: 0,
            parameter_drafts: BTreeMap::new(),
            selected_node: None,
            pending_source: None,
            drag: None,
            edit_status: "canonical workspace ready; no structural edits".to_owned(),
            persistence_dirty: persisted.is_none(),
            persistence_attempted: false,
            file_status: "canonical workspace has not been exported this session".to_owned(),
            file_bridge: WorkspaceFileBridge::default(),
            cursor_tick: 0,
        };
        if let Some(persisted) = persisted {
            match decode_persisted_workspace(persisted, result.workspace.limits())
                .and_then(|bytes| result.restore_workspace_bytes(&bytes))
            {
                Ok(()) => {
                    "restored canonical ALGW from application storage"
                        .clone_into(&mut result.edit_status);
                }
                Err(error) => {
                    result.persistence_dirty = true;
                    result.edit_status = format!(
                        "persisted ALGW rejected; canonical reference loaded instead: {error}"
                    );
                }
            }
        }
        Ok(result)
    }

    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) const fn persistence_pending(&self) -> bool {
        self.persistence_dirty && !self.persistence_attempted
    }

    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn persisted_workspace(&self) -> Result<String, String> {
        encode_persisted_workspace(&self.workspace_encoding)
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn mark_persisted(&mut self) {
        self.persistence_dirty = false;
        self.persistence_attempted = false;
    }

    pub(crate) fn note_persistence_error(&mut self, error: &str) {
        self.persistence_attempted = true;
        self.file_status = format!("browser persistence failed: {error}");
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
        if let Some(component) = &self.component {
            ui.label(format!(
                "Component: {} v{} · {} outputs / {} panel items · {} bytes",
                component.document.name(),
                component.document.component_version(),
                component.document.outputs().len(),
                component.document.panel_items().len(),
                component.encoding.bytes().len()
            ));
            ui.monospace(format!(
                "component {}…",
                digest_prefix(component.encoding.digest().0)
            ));
        }
        ui.label(format!(
            "History: {} undo / {} redo · {} bytes",
            self.history.undo_len(),
            self.history.redo_len(),
            self.history.retained_bytes()
        ));
        #[cfg(target_arch = "wasm32")]
        ui.label(if self.persistence_dirty && self.persistence_attempted {
            "Browser persistence: failed; edit to retry"
        } else if self.persistence_dirty {
            "Browser persistence: pending"
        } else {
            "Browser persistence: canonical bytes saved"
        });
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
        self.handle_history_shortcuts(ui);
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
                    "draft graph changed; reference replay detached"
                },
            );
            if ui.small_button("reset draft").clicked() {
                self.reset_draft();
            }
        });
        ui.label(
            "Add audited node kinds, drag headers onto the integer canvas, edit exact parameters, and connect typed ports in the in-memory ALGW draft. Secondary-click an input to disconnect. Editing never arms or commands firmware.",
        );
        self.show_workspace_controls(ui);
        self.show_palette(ui);
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

        self.show_front_panel(ui);
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

    fn handle_history_shortcuts(&mut self, ui: &egui::Ui) {
        if ui.ctx().wants_keyboard_input() {
            return;
        }
        let (undo, redo) = ui.input(|input| {
            let command = input.modifiers.command;
            let undo = command && !input.modifiers.shift && input.key_pressed(egui::Key::Z);
            let redo = command
                && (input.key_pressed(egui::Key::Y)
                    || (input.modifiers.shift && input.key_pressed(egui::Key::Z)));
            (undo, redo)
        });
        if undo {
            self.navigate_history(false);
        } else if redo {
            self.navigate_history(true);
        }
    }

    fn show_workspace_controls(&mut self, ui: &mut egui::Ui) {
        let mut navigate = None;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(self.history.can_undo(), egui::Button::new("undo"))
                .clicked()
            {
                navigate = Some(false);
            }
            if ui
                .add_enabled(self.history.can_redo(), egui::Button::new("redo"))
                .clicked()
            {
                navigate = Some(true);
            }
            ui.weak(format!(
                "{} back / {} forward · {} retained bytes",
                self.history.undo_len(),
                self.history.redo_len(),
                self.history.retained_bytes()
            ));
        });
        if let Some(redo) = navigate {
            self.navigate_history(redo);
        }

        let download_name = format!(
            "alumina-{}.algw",
            digest_prefix(self.workspace_encoding.digest().0)
        );
        let events = self.file_bridge.show(
            ui,
            self.workspace_encoding.bytes(),
            GraphWorkspaceLimits::interactive().maximum_workspace_bytes,
            &download_name,
        );
        for event in events {
            match event {
                WorkspaceFileEvent::Import(Ok(bytes)) => {
                    match self.import_workspace_bytes(&bytes) {
                        Ok(()) => {
                            self.file_status = format!(
                                "imported {} canonical ALGW bytes after full replay",
                                bytes.len()
                            );
                        }
                        Err(error) => {
                            self.file_status =
                                format!("ALGW import rejected without mutation: {error}");
                        }
                    }
                }
                WorkspaceFileEvent::Import(Err(error)) => {
                    self.file_status = format!("ALGW file read rejected: {error}");
                }
                WorkspaceFileEvent::Export(Ok(bytes)) => {
                    self.file_status = format!("exported {bytes} exact canonical ALGW bytes");
                }
                WorkspaceFileEvent::Export(Err(error)) => {
                    self.file_status = format!("ALGW export failed: {error}");
                }
            }
        }
        ui.weak(&self.file_status);
    }

    fn navigate_history(&mut self, redo: bool) {
        let admission = GraphWorkspaceLimits::interactive();
        let graph_admission = GraphLimits::interactive();
        let preview = if redo {
            self.history.preview_redo(admission, graph_admission)
        } else {
            self.history.preview_undo(admission, graph_admission)
        };
        let Some(preview) = (match preview {
            Ok(preview) => preview,
            Err(error) => {
                self.edit_status = format!("history replay rejected without mutation: {error}");
                return;
            }
        }) else {
            self.edit_status = if redo {
                "no later canonical workspace is retained".to_owned()
            } else {
                "no prior canonical workspace is retained".to_owned()
            };
            return;
        };
        let candidate = preview.document().clone();
        let (encoding, presentation, semantic) = match self.prepare_candidate(&candidate) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.edit_status = format!("history target rejected without mutation: {error}");
                return;
            }
        };
        let navigation = if redo {
            self.history
                .redo(&self.workspace, admission, graph_admission)
        } else {
            self.history
                .undo(&self.workspace, admission, graph_admission)
        };
        let replay = match navigation {
            Ok(Some(replay)) => replay,
            Ok(None) => {
                "history target disappeared without mutation".clone_into(&mut self.edit_status);
                return;
            }
            Err(error) => {
                self.edit_status = format!("history navigation rejected without mutation: {error}");
                return;
            }
        };
        debug_assert_eq!(replay.encoding(), &encoding);
        self.workspace = replay.into_document();
        self.workspace_encoding = encoding;
        self.presentation = presentation;
        self.pending_source = None;
        self.drag = None;
        self.parameter_drafts.clear();
        self.selected_node = self
            .selected_node
            .filter(|node| self.workspace.graph().node(*node).is_some());
        self.persistence_dirty = true;
        self.persistence_attempted = false;
        self.refresh_component();
        self.edit_status = format!(
            "{} canonical workspace; {semantic}",
            if redo { "redid" } else { "undid to" }
        );
    }

    fn show_palette(&mut self, ui: &mut egui::Ui) {
        let selected = self
            .palette
            .get(self.palette_index)
            .map_or("palette unavailable", |entry| entry.display_name.as_str());
        let mut add_requested = false;
        ui.horizontal_wrapped(|ui| {
            ui.strong("Audited node palette");
            egui::ComboBox::from_id_salt("exact_control_node_palette")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for (index, entry) in self.palette.iter().enumerate() {
                        ui.selectable_value(&mut self.palette_index, index, &entry.display_name);
                    }
                });
            add_requested = ui.small_button("add node").clicked();
            ui.weak(format!("{} fixed HostExact kinds", self.palette.len()));
        });
        if add_requested {
            self.add_palette_node();
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "canonical panel layout, exact control editing, and replay-only indicators remain one auditable egui frame operation"
    )]
    fn show_front_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Reusable component front panel");
            if let Some(component) = &self.component {
                ui.monospace(format!(
                    "ALGC {}… · {} exact bindings",
                    digest_prefix(component.encoding.digest().0),
                    component.document.panel_items().len()
                ));
            }
        });
        ui.label(
            "This first canonical component wraps the autonomous reference controller: exact parameter controls write ALGW through the normal transactional boundary, and output indicators read only the attached exact replay.",
        );
        let Some(component) = &self.component else {
            ui.colored_label(egui::Color32::YELLOW, &self.component_status);
            return;
        };
        let items = component
            .document
            .panel_items()
            .iter()
            .filter_map(|item| {
                let value_type = component.document.panel_item_value_type(item.id())?;
                let output_text = match item.binding() {
                    GraphFrontPanelBinding::OutputIndicator(output) => {
                        Some(self.component_output_text(&component.document, output))
                    }
                    GraphFrontPanelBinding::InputControl(_)
                    | GraphFrontPanelBinding::ParameterControl { .. } => None,
                };
                Some(ComponentPanelItem {
                    name: item.name().to_owned(),
                    binding: item.binding(),
                    rect: item.rect(),
                    value_type,
                    output_text,
                })
            })
            .collect::<Vec<_>>();
        let maximum_right = items
            .iter()
            .map(|item| {
                u32::try_from(item.rect.x()).expect("validated ALGC panel x is nonnegative")
                    + item.rect.width()
            })
            .max()
            .unwrap_or(0);
        let maximum_bottom = items
            .iter()
            .map(|item| {
                u32::try_from(item.rect.y()).expect("validated ALGC panel y is nonnegative")
                    + item.rect.height()
            })
            .max()
            .unwrap_or(0);
        let panel_size = egui::vec2(
            display_panel_coordinate(maximum_right.saturating_add(20)).max(360.0),
            display_panel_coordinate(maximum_bottom.saturating_add(20)).max(120.0),
        );
        let mut parameter_request = None;
        egui::ScrollArea::both()
            .id_salt("exact_control_component_front_panel")
            .auto_shrink([false, false])
            .max_height(285.0)
            .show(ui, |ui| {
                let (surface, painter) = ui.allocate_painter(panel_size, egui::Sense::hover());
                painter.rect_filled(surface.rect, 5.0, egui::Color32::from_rgb(21, 25, 34));
                paint_grid(&painter, surface.rect);
                for item in &items {
                    let rect = egui::Rect::from_min_size(
                        surface.rect.min
                            + egui::vec2(
                                display_coordinate(item.rect.x()),
                                display_coordinate(item.rect.y()),
                            ),
                        egui::vec2(
                            display_panel_coordinate(item.rect.width()),
                            display_panel_coordinate(item.rect.height()),
                        ),
                    );
                    painter.rect_filled(rect, 5.0, egui::Color32::from_rgb(35, 44, 57));
                    painter.rect_stroke(
                        rect,
                        5.0,
                        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(90, 119, 151)),
                    );
                    painter.text(
                        rect.left_top() + egui::vec2(8.0, 6.0),
                        egui::Align2::LEFT_TOP,
                        panel_item_label(&item.name),
                        egui::FontId::proportional(12.0),
                        egui::Color32::WHITE,
                    );
                    match item.binding {
                        GraphFrontPanelBinding::ParameterControl { node, parameter } => {
                            let Some(parameter_definition) = self
                                .workspace
                                .graph()
                                .node(node)
                                .and_then(|node| {
                                    node.parameters()
                                        .iter()
                                        .find(|candidate| candidate.id() == parameter)
                                })
                                .cloned()
                            else {
                                continue;
                            };
                            let Some(initial) =
                                parameter_edit_text(parameter_definition.value().value())
                            else {
                                painter.text(
                                    rect.center_bottom() - egui::vec2(0.0, 7.0),
                                    egui::Align2::CENTER_BOTTOM,
                                    "exact composite control is not rendered yet",
                                    egui::FontId::monospace(9.5),
                                    egui::Color32::YELLOW,
                                );
                                continue;
                            };
                            let maximum = parameter_text_limit(
                                self.workspace.graph(),
                                parameter_definition.value().value_type(),
                            );
                            let draft = self
                                .parameter_drafts
                                .entry((node, parameter))
                                .or_insert(initial);
                            let input_rect = egui::Rect::from_min_max(
                                rect.left_top() + egui::vec2(8.0, 25.0),
                                rect.right_bottom() - egui::vec2(57.0, 7.0),
                            );
                            let button_rect = egui::Rect::from_min_max(
                                egui::pos2(input_rect.right() + 5.0, input_rect.top()),
                                rect.right_bottom() - egui::vec2(7.0, 7.0),
                            );
                            let response = ui.put(
                                input_rect,
                                egui::TextEdit::singleline(draft).char_limit(maximum),
                            );
                            let apply = ui.put(button_rect, egui::Button::new("apply")).clicked()
                                || (response.lost_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                            if apply {
                                parameter_request = Some((node, parameter, draft.clone()));
                            }
                        }
                        GraphFrontPanelBinding::OutputIndicator(_) => {
                            painter.text(
                                rect.left_bottom() + egui::vec2(8.0, -8.0),
                                egui::Align2::LEFT_BOTTOM,
                                item.output_text.as_deref().unwrap_or("no exact sample"),
                                egui::FontId::monospace(11.0),
                                if self.reference_trace_is_current() {
                                    egui::Color32::from_rgb(122, 211, 185)
                                } else {
                                    egui::Color32::YELLOW
                                },
                            );
                        }
                        GraphFrontPanelBinding::InputControl(_) => {
                            painter.text(
                                rect.left_bottom() + egui::vec2(8.0, -8.0),
                                egui::Align2::LEFT_BOTTOM,
                                format!("runtime input · t{}", item.value_type.get()),
                                egui::FontId::monospace(10.0),
                                egui::Color32::GRAY,
                            );
                        }
                    }
                }
            });
        ui.weak(&self.component_status);
        if let Some((node, parameter, text)) = parameter_request {
            self.commit_parameter_text(node, parameter, &text);
        }
    }

    fn component_output_text(
        &self,
        component: &GraphComponentDocument,
        output: GraphComponentOutputId,
    ) -> String {
        if !self.reference_trace_is_current() {
            return "exact replay detached after draft edit".to_owned();
        }
        let Some(endpoint) = component.output(output).map(GraphComponentOutput::source) else {
            return "unresolved output binding".to_owned();
        };
        let Some(signal) = SIGNALS
            .iter()
            .copied()
            .find(|signal| signal.endpoint() == endpoint)
        else {
            return format!(
                "output #{}.{} has no replay probe",
                endpoint.node.get(),
                endpoint.port.get()
            );
        };
        self.traces
            .iter()
            .find(|series| series.signal == signal)
            .and_then(|series| {
                series
                    .points
                    .iter()
                    .find(|point| point.tick == self.cursor_tick)
            })
            .map_or_else(
                || format!("no sample at tick {}", self.cursor_tick),
                |point| format!("{} mm · tick {}", point.exact, point.tick),
            )
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
            Ok((workspace, _, _)) => {
                if self.commit_candidate(
                    workspace,
                    "reset draft to the canonical reference graph and layout",
                ) {
                    self.selected_node = None;
                    self.pending_source = None;
                    self.drag = None;
                    self.parameter_drafts.clear();
                }
            }
            Err(error) => {
                self.edit_status = format!("draft reset failed without mutation: {error}");
            }
        }
    }

    fn add_palette_node(&mut self) {
        let Some(entry) = self.palette.get(self.palette_index).cloned() else {
            "node creation rejected without mutation: palette selection is unavailable"
                .clone_into(&mut self.edit_status);
            return;
        };
        let (x, y) = match new_node_position(&self.workspace) {
            Ok(position) => position,
            Err(error) => {
                self.edit_status = format!("node creation rejected without mutation: {error}");
                return;
            }
        };
        let mut candidate = self.workspace.clone();
        let id = match candidate.create_node(entry.prototype, x, y) {
            Ok(id) => id,
            Err(error) => {
                self.edit_status = format!("node creation rejected without mutation: {error}");
                return;
            }
        };
        if self.commit_candidate(
            candidate,
            &format!(
                "created {} as node {} at canonical canvas ({x}, {y})",
                entry.display_name,
                id.get()
            ),
        ) {
            self.selected_node = Some(id);
            self.pending_source = None;
        }
    }

    fn delete_selected_node(&mut self, id: GraphNodeId) {
        let mut candidate = self.workspace.clone();
        let removed_wires = match candidate.delete_node(id) {
            Ok(count) => count,
            Err(error) => {
                self.edit_status = format!("node deletion rejected without mutation: {error}");
                return;
            }
        };
        if self.commit_candidate(
            candidate,
            &format!(
                "deleted node {} and {removed_wires} incident wire(s) without reusing identities",
                id.get()
            ),
        ) {
            self.selected_node = None;
            self.pending_source = self.pending_source.filter(|source| source.node != id);
            self.parameter_drafts.retain(|(node, _), _| *node != id);
        }
    }

    fn commit_parameter_text(&mut self, node_id: GraphNodeId, parameter_id: u32, text: &str) {
        let Some(parameter) = self
            .workspace
            .graph()
            .node(node_id)
            .and_then(|node| {
                node.parameters()
                    .iter()
                    .find(|parameter| parameter.id() == parameter_id)
            })
            .cloned()
        else {
            self.edit_status = format!(
                "parameter edit rejected without mutation: node {} parameter {parameter_id} is unavailable",
                node_id.get()
            );
            return;
        };
        let value = match parse_parameter_text(
            self.workspace.graph(),
            parameter.value().value_type(),
            text,
        ) {
            Ok(value) => value,
            Err(error) => {
                self.edit_status = format!("parameter edit rejected without mutation: {error}");
                return;
            }
        };
        let canonical = parameter_edit_text(value.value()).unwrap_or_else(|| text.to_owned());
        let mut candidate = self.workspace.clone();
        if let Err(error) = candidate.set_parameter(node_id, parameter_id, value) {
            self.edit_status = format!("parameter edit rejected without mutation: {error}");
            return;
        }
        if self.commit_candidate(
            candidate,
            &format!(
                "set node {} parameter {} to exact canonical {canonical}",
                node_id.get(),
                parameter.name()
            ),
        ) {
            self.parameter_drafts
                .insert((node_id, parameter_id), canonical);
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
        if candidate == self.workspace {
            self.edit_status = format!("{success}; canonical workspace already matched");
            return true;
        }
        let (encoding, presentation, semantic) = match self.prepare_candidate(&candidate) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.edit_status = format!("edit rejected without mutation: {error}");
                return false;
            }
        };
        if let Err(error) = self.history.record(self.workspace_encoding.clone()) {
            self.edit_status = format!("edit history rejected without mutation: {error}");
            return false;
        }
        self.workspace = candidate;
        self.workspace_encoding = encoding;
        self.presentation = presentation;
        self.selected_node = self
            .selected_node
            .filter(|node| self.workspace.graph().node(*node).is_some());
        self.pending_source = self
            .pending_source
            .filter(|source| self.workspace.graph().node(source.node).is_some());
        self.parameter_drafts
            .retain(|(node, _), _| self.workspace.graph().node(*node).is_some());
        self.persistence_dirty = true;
        self.persistence_attempted = false;
        self.refresh_component();
        self.edit_status = format!("{success}; {semantic}");
        true
    }

    fn prepare_candidate(
        &self,
        candidate: &GraphWorkspaceDocument,
    ) -> Result<(CanonicalGraphWorkspaceEncoding, GraphPresentation, String), String> {
        let presentation = graph_presentation(
            candidate.graph(),
            self.fixture.registry(),
            Some(candidate.placements()),
        )?;
        let encoding = encode_graph_workspace(candidate).map_err(|error| error.to_string())?;
        let draft_analysis = analyze_graph_draft(
            candidate.graph(),
            self.fixture.registry().semantic_registry(),
        )
        .map_err(|error| format!("audited semantics rejected: {error}"))?;
        let semantic = if let Some(first) = draft_analysis.required_unconnected_inputs().first() {
            format!(
                "draft semantic blocker: {} required input(s) unconnected; first {first:?}",
                draft_analysis.required_unconnected_inputs().len()
            )
        } else {
            "audited semantics valid".to_owned()
        };
        Ok((encoding, presentation, semantic))
    }

    fn restore_workspace_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        let replay = replay_graph_workspace(
            bytes,
            GraphWorkspaceLimits::interactive(),
            GraphLimits::interactive(),
        )
        .map_err(|error| error.to_string())?;
        let candidate = replay.document().clone();
        let (encoding, presentation, _) = self.prepare_candidate(&candidate)?;
        if replay.encoding() != &encoding {
            return Err("replayed ALGW identity changed during UI admission".to_owned());
        }
        self.workspace = candidate;
        self.workspace_encoding = encoding;
        self.presentation = presentation;
        self.history.clear();
        self.selected_node = None;
        self.pending_source = None;
        self.drag = None;
        self.parameter_drafts.clear();
        self.persistence_dirty = false;
        self.persistence_attempted = false;
        self.refresh_component();
        Ok(())
    }

    fn refresh_component(&mut self) {
        match representative_component(&self.workspace) {
            Ok(component) => {
                self.component = Some(component);
                "canonical ALGC connector pane and front panel attached"
                    .clone_into(&mut self.component_status);
            }
            Err(error) => {
                self.component = None;
                self.component_status = format!(
                    "ALGC front panel detached from this draft without affecting ALGW: {error}"
                );
            }
        }
    }

    fn import_workspace_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        let replay = replay_graph_workspace(
            bytes,
            GraphWorkspaceLimits::interactive(),
            GraphLimits::interactive(),
        )
        .map_err(|error| error.to_string())?;
        let digest = digest_prefix(replay.encoding().digest().0);
        if self.commit_candidate(
            replay.into_document(),
            &format!("imported canonical ALGW {digest}…"),
        ) {
            self.pending_source = None;
            self.drag = None;
            self.parameter_drafts.clear();
            Ok(())
        } else {
            Err(self.edit_status.clone())
        }
    }

    fn show_selected_node(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected_node else {
            ui.weak("Select a node to inspect exact ports, parameters, and state authority.");
            return;
        };
        let Some(node) = self.workspace.graph().node(id).cloned() else {
            return;
        };
        let document = self.workspace.graph().clone();
        let placement = self.workspace.placement(id);
        let state = self
            .fixture
            .registry()
            .semantic_registry()
            .schema(node.kind())
            .and_then(alumina_interface_core::graph::NodeSchema::state);
        let mut delete_requested = false;
        let mut parameter_request = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong(format!("#{} {}", id.get(), node.label()));
                ui.monospace(format!("{} v{}", node.kind().name(), node.kind().version()));
                if ui.small_button("clear").clicked() {
                    self.selected_node = None;
                }
                delete_requested = ui.small_button("delete node + wires").clicked();
            });
            ui.label(format!("Execution domain: {}", domain_label(node.domain())));
            if let Some(placement) = placement {
                ui.monospace(format!(
                    "canvas = ({}, {}) logical px · presentation only",
                    placement.x(),
                    placement.y()
                ));
            }
            ui.horizontal_wrapped(|ui| {
                for port in node.inputs() {
                    ui.monospace(port_description(&document, "in", port));
                }
                for port in node.outputs() {
                    ui.monospace(port_description(&document, "out", port));
                }
            });
            for parameter in node.parameters() {
                let Some(initial) = parameter_edit_text(parameter.value().value()) else {
                    ui.monospace(format!(
                        "{} = {} · read-only literal shape",
                        parameter.name(),
                        typed_value_text(&document, parameter.value())
                    ));
                    continue;
                };
                let maximum = parameter_text_limit(&document, parameter.value().value_type());
                let draft = self
                    .parameter_drafts
                    .entry((id, parameter.id()))
                    .or_insert(initial);
                ui.horizontal_wrapped(|ui| {
                    ui.monospace(format!("{}:", parameter.name()));
                    let response = ui.add(
                        egui::TextEdit::singleline(draft)
                            .desired_width(180.0)
                            .char_limit(maximum),
                    );
                    let apply = ui.small_button("apply exact").clicked()
                        || (response.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                    ui.weak(typed_value_text(&document, parameter.value()));
                    if apply {
                        parameter_request = Some((parameter.id(), draft.clone()));
                    }
                });
            }
            if let Some(state) = state {
                ui.label(format!(
                    "Explicit state: clock {}, t{}, read-before-write, ≤{} canonical bytes",
                    state.clock().get(),
                    state.value_type().get(),
                    state.declared_storage_bytes()
                ));
            }
        });
        if delete_requested {
            self.delete_selected_node(id);
        } else if let Some((parameter, text)) = parameter_request {
            self.commit_parameter_text(id, parameter, &text);
        }
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

fn control_palette(
    fixture: &RepresentativeExactControlGraph,
) -> Result<Vec<NodePaletteEntry>, String> {
    let registry = fixture.registry();
    let semantic = registry.semantic_registry();
    let mut palette = Vec::with_capacity(semantic.schemas().len());
    for schema in semantic.schemas() {
        if registry.implementation(schema.kind()).is_none() {
            return Err(format!(
                "audited palette kind {} v{} has no fixed simulation implementation",
                schema.kind().name(),
                schema.kind().version()
            ));
        }
        let exemplar = fixture
            .document()
            .nodes()
            .iter()
            .find(|node| node.kind() == schema.kind())
            .ok_or_else(|| {
                format!(
                    "audited palette kind {} v{} has no reviewed default instance",
                    schema.kind().name(),
                    schema.kind().version()
                )
            })?;
        if exemplar.inputs() != schema.inputs()
            || exemplar.outputs() != schema.outputs()
            || !schema.allowed_domains().contains(exemplar.domain())
            || !parameters_match_schema(exemplar.parameters(), schema.parameters())
        {
            return Err(format!(
                "audited palette kind {} v{} disagrees with its reviewed default instance",
                schema.kind().name(),
                schema.kind().version()
            ));
        }
        let short = short_kind(schema.kind().name());
        palette.push(NodePaletteEntry {
            display_name: format!("{short} v{}", schema.kind().version()),
            prototype: GraphNodePrototype::new(
                schema.kind().clone(),
                format!("New {short}"),
                exemplar.domain(),
                schema.inputs().to_vec(),
                schema.outputs().to_vec(),
                exemplar.parameters().to_vec(),
            ),
        });
    }
    if palette.is_empty() {
        return Err("audited node palette is empty".to_owned());
    }
    Ok(palette)
}

fn parameters_match_schema(
    parameters: &[NodeParameter],
    contracts: &[alumina_interface_core::graph::NodeParameterContract],
) -> bool {
    parameters.len() == contracts.len()
        && parameters
            .iter()
            .zip(contracts)
            .all(|(parameter, contract)| {
                parameter.id() == contract.id()
                    && parameter.name() == contract.name()
                    && parameter.value().value_type() == contract.value_type()
            })
}

fn new_node_position(workspace: &GraphWorkspaceDocument) -> Result<(i32, i32), String> {
    let Some(maximum_x) = workspace
        .placements()
        .iter()
        .map(|placement| placement.x())
        .max()
    else {
        return Ok((NEW_NODE_ORIGIN, NEW_NODE_ORIGIN));
    };
    let x = maximum_x
        .checked_add(NEW_NODE_X_GAP)
        .ok_or_else(|| "node palette position overflowed the canvas lattice".to_owned())?;
    if x.unsigned_abs() > workspace.limits().maximum_coordinate_magnitude {
        return Err("node palette position exceeds the canonical canvas policy".to_owned());
    }
    let y = workspace
        .placements()
        .iter()
        .map(|placement| placement.y())
        .min()
        .unwrap_or(NEW_NODE_ORIGIN);
    Ok((x, y))
}

fn parameter_edit_text(value: &GraphValue) -> Option<String> {
    match value {
        GraphValue::Boolean(value) => Some(value.to_string()),
        GraphValue::ExactRational(value) => Some(value.to_string()),
        GraphValue::MeasurementInterval { lower, upper } => Some(format!("{lower}..{upper}")),
        GraphValue::CanonicalI64(value) => Some(value.to_string()),
        GraphValue::CanonicalU64(value) => Some(value.to_string()),
        GraphValue::Text(value) => Some(value.clone()),
        GraphValue::Bytes(_)
        | GraphValue::Array(_)
        | GraphValue::Record(_)
        | GraphValue::OptionNone
        | GraphValue::OptionSome(_)
        | GraphValue::ResultOk(_)
        | GraphValue::ResultError(_)
        | GraphValue::ResourceHandle(_)
        | GraphValue::JobHandle(_) => None,
    }
}

fn parameter_text_limit(document: &GraphDocument, value_type: GraphTypeId) -> usize {
    let rational_digits = document.schema().limits().maximum_rational_digits;
    document
        .schema()
        .value_type(value_type)
        .map_or(1, |definition| match definition.kind() {
            TypeKind::Boolean => 5,
            TypeKind::ExactRational { .. } => rational_digits.saturating_mul(2).saturating_add(4),
            TypeKind::MeasurementInterval { .. } => {
                rational_digits.saturating_mul(4).saturating_add(10)
            }
            TypeKind::CanonicalI64 { .. } | TypeKind::CanonicalU64 { .. } => 20,
            TypeKind::Text { maximum_bytes } => *maximum_bytes as usize,
            TypeKind::Bytes { .. }
            | TypeKind::Array { .. }
            | TypeKind::Record { .. }
            | TypeKind::Option { .. }
            | TypeKind::Result { .. }
            | TypeKind::Event { .. }
            | TypeKind::Stream { .. }
            | TypeKind::ResourceHandle { .. }
            | TypeKind::JobHandle => 1,
        })
}

fn parse_parameter_text(
    document: &GraphDocument,
    value_type: GraphTypeId,
    source: &str,
) -> Result<TypedGraphValue, String> {
    let maximum = parameter_text_limit(document, value_type);
    if source.len() > maximum {
        return Err(format!(
            "parameter text has {} bytes; this exact type admits at most {maximum}",
            source.len()
        ));
    }
    let definition = document
        .schema()
        .value_type(value_type)
        .ok_or_else(|| format!("parameter type {} is not registered", value_type.get()))?;
    let trimmed = source.trim();
    let value = match definition.kind() {
        TypeKind::Boolean => match trimmed {
            "true" => GraphValue::Boolean(true),
            "false" => GraphValue::Boolean(false),
            _ => return Err("Boolean parameter must be exactly true or false".to_owned()),
        },
        TypeKind::ExactRational { .. } => GraphValue::ExactRational(
            trimmed
                .parse::<Rational>()
                .map_err(|error| format!("exact rational parameter is invalid: {error}"))?,
        ),
        TypeKind::MeasurementInterval { .. } => {
            let (lower, upper) = trimmed
                .split_once("..")
                .ok_or_else(|| "measurement interval must use lower..upper".to_owned())?;
            GraphValue::MeasurementInterval {
                lower: lower
                    .trim()
                    .parse::<Rational>()
                    .map_err(|error| format!("measurement lower bound is invalid: {error}"))?,
                upper: upper
                    .trim()
                    .parse::<Rational>()
                    .map_err(|error| format!("measurement upper bound is invalid: {error}"))?,
            }
        }
        TypeKind::CanonicalI64 { .. } => GraphValue::CanonicalI64(
            trimmed
                .parse::<i64>()
                .map_err(|error| format!("canonical signed count is invalid: {error}"))?,
        ),
        TypeKind::CanonicalU64 { .. } => GraphValue::CanonicalU64(
            trimmed
                .parse::<u64>()
                .map_err(|error| format!("canonical unsigned count is invalid: {error}"))?,
        ),
        TypeKind::Text { .. } => GraphValue::Text(source.to_owned()),
        TypeKind::Bytes { .. }
        | TypeKind::Array { .. }
        | TypeKind::Record { .. }
        | TypeKind::Option { .. }
        | TypeKind::Result { .. }
        | TypeKind::Event { .. }
        | TypeKind::Stream { .. }
        | TypeKind::ResourceHandle { .. }
        | TypeKind::JobHandle => {
            return Err(format!(
                "parameter type {} is not editable in this scalar palette slice",
                definition.name()
            ));
        }
    };
    TypedGraphValue::try_new(document.schema(), value_type, value)
        .map_err(|error| format!("exact parameter failed schema validation: {error}"))
}

#[cfg(any(target_arch = "wasm32", test))]
fn encode_persisted_workspace(
    encoding: &CanonicalGraphWorkspaceEncoding,
) -> Result<String, String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let bytes = encoding.bytes();
    if bytes.len() > MAXIMUM_PERSISTED_WORKSPACE_BYTES {
        return Err(format!(
            "canonical ALGW has {} bytes; browser persistence admits at most {}",
            bytes.len(),
            MAXIMUM_PERSISTED_WORKSPACE_BYTES
        ));
    }
    let encoded_bytes = bytes
        .len()
        .checked_mul(2)
        .and_then(|length| length.checked_add(PERSISTED_WORKSPACE_PREFIX.len()))
        .ok_or_else(|| "persisted ALGW text length overflowed".to_owned())?;
    let mut result = String::with_capacity(encoded_bytes);
    result.push_str(PERSISTED_WORKSPACE_PREFIX);
    for byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(result)
}

fn decode_persisted_workspace(
    value: &str,
    workspace_limits: GraphWorkspaceLimits,
) -> Result<Vec<u8>, String> {
    let encoded = value
        .strip_prefix(PERSISTED_WORKSPACE_PREFIX)
        .ok_or_else(|| "persisted ALGW prefix/version is unsupported".to_owned())?;
    if !encoded.len().is_multiple_of(2) {
        return Err("persisted ALGW hex length is odd".to_owned());
    }
    let maximum_bytes = workspace_limits
        .maximum_workspace_bytes
        .min(MAXIMUM_PERSISTED_WORKSPACE_BYTES);
    let byte_length = encoded.len() / 2;
    if byte_length > maximum_bytes {
        return Err(format!(
            "persisted ALGW exceeds the {maximum_bytes}-byte admission limit"
        ));
    }
    let mut bytes = Vec::with_capacity(byte_length);
    for pair in encoded.as_bytes().chunks_exact(2) {
        let high = canonical_hex_nibble(pair[0])
            .ok_or_else(|| "persisted ALGW is not canonical lowercase hex".to_owned())?;
        let low = canonical_hex_nibble(pair[1])
            .ok_or_else(|| "persisted ALGW is not canonical lowercase hex".to_owned())?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

const fn canonical_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
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

fn representative_component(
    workspace: &GraphWorkspaceDocument,
) -> Result<ComponentPackage, String> {
    let outputs = SIGNALS
        .iter()
        .copied()
        .enumerate()
        .map(|(index, signal)| {
            let id = u32::try_from(index + 1)
                .map_err(|_| "representative output identity overflowed".to_owned())?;
            let name = match signal {
                RepresentativeControlSignal::Error => "error",
                RepresentativeControlSignal::IntegralPrior => "integral_prior",
                RepresentativeControlSignal::ClampedController => "clamped_controller",
                RepresentativeControlSignal::PermittedOutput => "permitted_output",
            };
            Ok(GraphComponentOutput::new(
                GraphComponentOutputId::new(id),
                name,
                signal.endpoint(),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let parameter_specs = [
        (1, "proportional_gain", 8, 1, 20, 20),
        (2, "integral_gain", 11, 1, 20, 84),
        (3, "derivative_gain", 14, 1, 20, 148),
        (4, "output_minimum", 17, 1, 240, 20),
        (5, "output_maximum", 17, 2, 240, 84),
        (6, "safe_output", 18, 1, 240, 148),
    ];
    let mut panel_items = parameter_specs
        .into_iter()
        .map(|(id, name, node, parameter, x, y)| {
            GraphFrontPanelItem::new(
                GraphFrontPanelItemId::new(id),
                name,
                GraphFrontPanelBinding::ParameterControl {
                    node: GraphNodeId::new(node),
                    parameter,
                },
                GraphFrontPanelRect::new(x, y, 200, 54),
            )
        })
        .collect::<Vec<_>>();
    let output_specs = [
        (7, "error_indicator", 1, 20),
        (8, "integral_prior_indicator", 2, 84),
        (9, "clamped_controller_indicator", 3, 148),
        (10, "permitted_output_indicator", 4, 212),
    ];
    panel_items.extend(output_specs.into_iter().map(|(id, name, output, y)| {
        GraphFrontPanelItem::new(
            GraphFrontPanelItemId::new(id),
            name,
            GraphFrontPanelBinding::OutputIndicator(GraphComponentOutputId::new(output)),
            GraphFrontPanelRect::new(460, y, 240, 54),
        )
    }));
    let document = GraphComponentDocument::try_new(
        GraphComponentLimits::interactive(),
        workspace.revision(),
        1,
        "control.reference_pid",
        1,
        5,
        11,
        workspace.clone(),
        Vec::new(),
        outputs,
        panel_items,
    )
    .map_err(|error| error.to_string())?;
    let encoding = encode_graph_component(&document).map_err(|error| error.to_string())?;
    Ok(ComponentPackage { document, encoding })
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
        if placements.is_some_and(|placements| !placements.is_empty()) {
            return Err("empty exact control graph retained canvas placements".to_owned());
        }
        return Ok(GraphPresentation {
            nodes: BTreeMap::new(),
            wires: BTreeMap::new(),
            size: egui::vec2(EMPTY_CANVAS_WIDTH, EMPTY_CANVAS_HEIGHT),
        });
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

fn panel_item_label(name: &str) -> String {
    name.replace('_', " ")
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
    clippy::cast_precision_loss,
    reason = "front-panel coordinates are bounded to one million and exactly representable in display f32"
)]
fn display_panel_coordinate(value: u32) -> f32 {
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
    use alumina_interface_core::graph::{
        GraphLimits, GraphPortId, NodeKind, replay_graph_component, replay_graph_workspace,
    };

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
        assert_eq!(workspace.palette.len(), 11);
        assert!(workspace.palette.iter().all(|entry| {
            workspace
                .fixture
                .registry()
                .semantic_registry()
                .schema(entry.prototype.kind())
                .is_some()
        }));
    }

    #[test]
    fn canonical_component_panel_tracks_exact_edits_and_detaches_transactionally() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial = workspace.component.as_ref().unwrap();
        assert_eq!(initial.encoding.bytes().len(), 4_099);
        assert_eq!(
            initial.encoding.digest().0,
            [
                0x20, 0x75, 0x9f, 0xd4, 0x76, 0xc4, 0x35, 0xec, 0xa5, 0x31, 0x82, 0x04, 0xc1, 0x04,
                0x8e, 0xed, 0x81, 0x56, 0x24, 0x4f, 0x73, 0x9b, 0xb2, 0xdf, 0x54, 0x48, 0xa7, 0xf5,
                0x80, 0xd3, 0x59, 0xd1,
            ]
        );
        assert!(initial.document.inputs().is_empty());
        assert_eq!(initial.document.outputs().len(), 4);
        assert_eq!(initial.document.panel_items().len(), 10);
        assert_eq!(
            initial.document.workspace_digest(),
            workspace.workspace_encoding.digest()
        );
        let replay = replay_graph_component(
            initial.encoding.bytes(),
            GraphComponentLimits::interactive(),
            GraphWorkspaceLimits::interactive(),
            GraphLimits::interactive(),
        )
        .unwrap();
        assert_eq!(replay.document(), &initial.document);
        assert_eq!(replay.encoding(), &initial.encoding);

        let initial_digest = initial.encoding.digest();
        workspace.commit_parameter_text(GraphNodeId::new(8), 1, "201");
        let edited = workspace.component.as_ref().unwrap();
        assert_ne!(edited.encoding.digest(), initial_digest);
        assert_eq!(
            edited.document.workspace_digest(),
            workspace.workspace_encoding.digest()
        );

        workspace.delete_selected_node(GraphNodeId::new(18));
        assert!(workspace.component.is_none());
        assert!(workspace.component_status.contains("detached"));
        workspace.navigate_history(false);
        assert!(workspace.component.is_some());
        assert_eq!(
            workspace
                .component
                .as_ref()
                .unwrap()
                .document
                .workspace_digest(),
            workspace.workspace_encoding.digest()
        );
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
    fn palette_node_lifecycle_and_exact_parameter_editing_are_transactional() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        workspace.palette_index = workspace
            .palette
            .iter()
            .position(|entry| entry.prototype.kind().name() == "control.exact.scale")
            .unwrap();
        workspace.add_palette_node();
        let created = GraphNodeId::new(20);
        assert_eq!(workspace.selected_node, Some(created));
        assert_eq!(workspace.workspace.graph().nodes().len(), 20);
        assert_eq!(workspace.workspace.next_node_id(), 21);
        assert_eq!(
            workspace
                .workspace
                .graph()
                .node(created)
                .unwrap()
                .kind()
                .name(),
            "control.exact.scale"
        );
        assert!(!workspace.reference_trace_is_current());
        assert!(workspace.edit_status.contains("draft semantic blocker"));

        workspace.delete_selected_node(created);
        assert_eq!(workspace.workspace.graph().nodes().len(), 19);
        assert_eq!(workspace.workspace.next_node_id(), 21);
        assert_eq!(workspace.selected_node, None);

        workspace.reset_draft();
        workspace.commit_parameter_text(GraphNodeId::new(8), 1, "3/2");
        assert_eq!(
            workspace
                .workspace
                .graph()
                .node(GraphNodeId::new(8))
                .unwrap()
                .parameters()[0]
                .value()
                .value(),
            &GraphValue::ExactRational(Rational::fraction(3, 2).unwrap())
        );
        let canonical_three_halves = Rational::fraction(3, 2).unwrap().to_string();
        assert_eq!(
            workspace
                .parameter_drafts
                .get(&(GraphNodeId::new(8), 1))
                .map(String::as_str),
            Some(canonical_three_halves.as_str())
        );
        assert!(!workspace.reference_trace_is_current());

        let retained = workspace.workspace.clone();
        workspace.commit_parameter_text(GraphNodeId::new(8), 1, "1/0");
        assert_eq!(workspace.workspace, retained);
        assert!(workspace.edit_status.contains("rejected without mutation"));
    }

    #[test]
    fn empty_draft_remains_renderable_and_can_accept_a_palette_node() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let mut candidate = workspace.workspace.clone();
        let ids: Vec<_> = candidate
            .graph()
            .nodes()
            .iter()
            .map(NodeDefinition::id)
            .collect();
        for id in ids {
            candidate.delete_node(id).unwrap();
        }
        assert!(workspace.commit_candidate(candidate, "deleted every draft node"));
        assert!(workspace.workspace.graph().nodes().is_empty());
        assert_eq!(workspace.presentation.nodes.len(), 0);
        assert!((workspace.presentation.size.x - EMPTY_CANVAS_WIDTH).abs() < f32::EPSILON);

        workspace.add_palette_node();
        assert_eq!(workspace.workspace.graph().nodes().len(), 1);
        assert_eq!(workspace.selected_node, Some(GraphNodeId::new(20)));
        assert_eq!(
            workspace.workspace.placement(GraphNodeId::new(20)),
            Some(GraphNodePlacement::new(
                GraphNodeId::new(20),
                NEW_NODE_ORIGIN,
                NEW_NODE_ORIGIN,
            ))
        );
    }

    #[test]
    fn canonical_history_drives_ui_undo_redo_and_clears_abandoned_redo() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        let initial = workspace.workspace.clone();
        let origin = workspace.workspace.placement(GraphNodeId::new(1)).unwrap();
        workspace.commit_node_drag(
            NodeDrag {
                node: GraphNodeId::new(1),
                origin,
            },
            egui::vec2(20.0, 30.0),
        );
        let moved = workspace.workspace.clone();
        assert_eq!(workspace.history.undo_len(), 1);
        assert_eq!(workspace.history.redo_len(), 0);

        workspace.navigate_history(false);
        assert_eq!(workspace.workspace, initial);
        assert_eq!(workspace.history.undo_len(), 0);
        assert_eq!(workspace.history.redo_len(), 1);
        workspace.navigate_history(true);
        assert_eq!(workspace.workspace, moved);

        workspace.navigate_history(false);
        let second_origin = workspace.workspace.placement(GraphNodeId::new(2)).unwrap();
        workspace.commit_node_drag(
            NodeDrag {
                node: GraphNodeId::new(2),
                origin: second_origin,
            },
            egui::vec2(-11.0, 7.0),
        );
        assert_eq!(workspace.history.redo_len(), 0);
        assert!(workspace.persistence_pending());
    }

    #[test]
    fn persistence_round_trips_only_current_canonical_workspace() {
        let mut workspace = ExactControlWorkspace::try_new().unwrap();
        workspace.commit_parameter_text(GraphNodeId::new(8), 1, "7/3");
        assert_eq!(workspace.history.undo_len(), 1);
        let persisted = workspace.persisted_workspace().unwrap();
        assert!(persisted.starts_with(PERSISTED_WORKSPACE_PREFIX));
        assert!(
            persisted[PERSISTED_WORKSPACE_PREFIX.len()..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );

        let restored = ExactControlWorkspace::try_new_with_persisted(Some(&persisted)).unwrap();
        assert_eq!(restored.workspace, workspace.workspace);
        assert_eq!(restored.workspace_encoding, workspace.workspace_encoding);
        assert_eq!(restored.history.undo_len(), 0);
        assert_eq!(restored.history.redo_len(), 0);
        assert!(!restored.persistence_pending());

        let mut uppercase = persisted.clone();
        let hex = &persisted[PERSISTED_WORKSPACE_PREFIX.len()..];
        let offset = hex
            .bytes()
            .position(|byte| (b'a'..=b'f').contains(&byte))
            .unwrap();
        let persisted_offset = PERSISTED_WORKSPACE_PREFIX.len() + offset;
        uppercase.replace_range(
            persisted_offset..=persisted_offset,
            &hex[offset..=offset].to_ascii_uppercase(),
        );
        assert!(
            decode_persisted_workspace(&uppercase, GraphWorkspaceLimits::interactive())
                .unwrap_err()
                .contains("lowercase hex")
        );
        assert!(
            decode_persisted_workspace(
                "algw1:0000",
                GraphWorkspaceLimits {
                    maximum_workspace_bytes: 1,
                    ..GraphWorkspaceLimits::interactive()
                },
            )
            .unwrap_err()
            .contains("admission limit")
        );
    }

    #[test]
    fn invalid_persistence_and_imports_fail_closed_without_losing_the_draft() {
        let fallback = ExactControlWorkspace::try_new_with_persisted(Some("wrong:00")).unwrap();
        assert!(fallback.edit_status.contains("persisted ALGW rejected"));
        assert!(fallback.persistence_pending());
        assert!(fallback.reference_trace_is_current());

        let mut source = ExactControlWorkspace::try_new().unwrap();
        source.commit_parameter_text(GraphNodeId::new(8), 1, "9/4");
        let imported_bytes = source.workspace_encoding.bytes().to_vec();
        let mut target = ExactControlWorkspace::try_new().unwrap();
        target.import_workspace_bytes(&imported_bytes).unwrap();
        assert_eq!(target.workspace, source.workspace);
        assert_eq!(target.history.undo_len(), 1);

        let retained_workspace = target.workspace.clone();
        let retained_history = target.history.clone();
        let mut corrupt = imported_bytes;
        corrupt[0] ^= 0xff;
        assert!(target.import_workspace_bytes(&corrupt).is_err());
        assert_eq!(target.workspace, retained_workspace);
        assert_eq!(target.history, retained_history);

        let exemplar = target
            .workspace
            .graph()
            .node(GraphNodeId::new(8))
            .unwrap()
            .clone();
        let mut unknown = target.workspace.clone();
        let required_wire = unknown
            .graph()
            .wires()
            .iter()
            .find(|wire| {
                wire.target()
                    == WireEndpoint {
                        node: GraphNodeId::new(19),
                        port: GraphPortId::new(1),
                    }
            })
            .unwrap()
            .id();
        unknown.disconnect(required_wire).unwrap();
        unknown
            .create_node(
                GraphNodePrototype::new(
                    NodeKind::new("control.unreviewed", 1),
                    "Unreviewed",
                    exemplar.domain(),
                    exemplar.inputs().to_vec(),
                    exemplar.outputs().to_vec(),
                    exemplar.parameters().to_vec(),
                ),
                9_000,
                100,
            )
            .unwrap();
        let unknown = encode_graph_workspace(&unknown).unwrap();
        assert!(
            target
                .import_workspace_bytes(unknown.bytes())
                .unwrap_err()
                .contains("audited semantics rejected")
        );
        assert_eq!(target.workspace, retained_workspace);
        assert_eq!(target.history, retained_history);
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
