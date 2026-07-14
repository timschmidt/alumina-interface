#![warn(clippy::pedantic)]
mod design_graph;
mod renderer;
mod fonts;

use crate::design_graph::{AllTemplates, UserState};
use csgrs::{mesh::Mesh, sketch::Sketch, traits::CSG};
use eframe::egui;
use egui_node_graph2::GraphEditorState;
use futures_channel::oneshot;
use geo::{Geometry, LineString};
use glow::HasContext as _;
use js_sys::Uint8Array;
use log::Level;
use nalgebra::{Matrix4, Perspective3, Point3, Translation3, UnitQuaternion, Vector3};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    f32::consts::{FRAC_PI_2, PI},
    future::Future,
    rc::Rc,
    sync::{Arc, Mutex},
};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Event, HtmlCanvasElement, HtmlInputElement, window};

const INVALID_SCALE: Vector3<f32> = Vector3::new(-1.0, -1.0, -1.0);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Laser,
    Plasma,
    Extruder,
    Endmill,
    Drill,
    DlpLcd,
}

impl std::fmt::Display for Tool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use Tool::*;
        write!(
            f,
            "{}",
            match self {
                Laser => "Laser",
                Plasma => "Plasma",
                Extruder => "Extruder",
                Endmill => "Endmill",
                Drill => "Drill",
                DlpLcd => "DLP / LCD",
            }
        )
    }
}

/// A single user-loaded model (STL/DXF) plus its per-instance transforms.
#[derive(Clone)]
struct ModelEntry {
    /// File-name or synthesized label shown in the sidebar list.
    name: String,
    /// Imported geometry before per-instance scale and translation.
    base: Mesh<()>,
    /// Rendered copy after scale and translation.
    mesh: Mesh<()>,
    /// Desired user scale and last-applied scale (so we can lazily rebuild).
    scale: Vector3<f32>,
    applied_scale: Vector3<f32>,
    /// Desired user offset (mm) and last-applied offset.
    offset: Vector3<f32>,
    applied_offset: Vector3<f32>,
}

impl ModelEntry {
    fn new(name: impl Into<String>, base: Mesh<()>) -> Self {
        Self {
            name: name.into(),
            scale: Vector3::new(1.0, 1.0, 1.0),
            applied_scale: Vector3::new(1.0, 1.0, 1.0),
            offset: Vector3::zeros(),
            applied_offset: Vector3::zeros(),
            mesh: base.clone(),
            base,
        }
    }

    /// Rebuild the rendered mesh after a scale or translation change.
    fn refresh(&mut self) {
        if self.scale != self.applied_scale || self.offset != self.applied_offset {
            self.mesh = self
                .base
                .clone()
                .scale(
                    self.scale.x.into(),
                    self.scale.y.into(),
                    self.scale.z.into(),
                )
                .translate(
                    self.offset.x.into(),
                    self.offset.y.into(),
                    self.offset.z.into(),
                );
            self.applied_scale = self.scale;
            self.applied_offset = self.offset;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InfillType {
    Linear,
    Gyroid,
    SchwarzP,
    SchwarzD,
}

impl std::fmt::Display for InfillType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use InfillType::*;
        write!(
            f,
            "{}",
            match self {
                Linear => "Linear",
                Gyroid => "Gyroid",
                SchwarzP => "Schwarz P",
                SchwarzD => "Schwarz D",
            }
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Control,
    Diagnostics,
    Design,
}

pub struct AluminaApp {
    rotation: UnitQuaternion<f32>,
    translation: egui::Vec2,
    zoom: f32,
    /// All user-loaded models (plus the default one).
    models: Vec<ModelEntry>,
    /// Model currently selected in the sidebar.
    selected_model: Option<usize>,
    workpiece_data: Arc<Mutex<Option<Vec<u8>>>>,
    model_data: Arc<Mutex<Option<Vec<u8>>>>,
    wireframe: bool,
    edges: bool,
    faces: bool,
    normals: bool,
    vertices: bool,
    workarea: bool,
    /// CNC work-envelope dimensions in millimeters.
    work_size: Vector3<f32>,
    layer_height: f32,
    /// Zero-based index of the layer currently being inspected.
    current_layer: i32,
    /// Whether the slice view is active.
    show_slice: bool,
    /// Most recently generated slice for `current_layer`.
    sliced_layer: Option<Sketch<()>>,
    gpu: Option<Arc<Mutex<renderer::GpuLines>>>,
    gpu_faces: Option<Arc<Mutex<renderer::GpuLines>>>,
    vertex_storage: Vec<f32>,
    selected_tab: Tab,
    diag_poll: bool,
    diag_led: bool,
    // Desired GPIO states; false means low.
    diag_d0:bool,diag_d1:bool,diag_d2:bool,diag_d3:bool,
    diag_d4:bool,diag_d5:bool,diag_d6:bool,diag_d7:bool,
    diag_d9:bool,diag_d11:bool,diag_d12:bool,diag_d13:bool,
    device_info_slot: Arc<Mutex<Option<(String /*name*/, String /*display_name*/, String /*image_mime*/, String /*image_url*/)>>>,
    device_info_requested: bool,
    selected_tool: Tool,
    kerf: f32,
    touch_off: bool,
    perimeters: i32,
    infill_type: InfillType,
    endmill_width: f32,
    endmill_length: f32,
    drill_width: f32,
    drill_length: f32,
    pixels_wide: i32,
    pixels_tall: i32,
    layer_delay: f32,
    peel_distance: f32,
    design_state: GraphEditorState<
        design_graph::NodeData,
        design_graph::DType,
        design_graph::DValue,
        design_graph::Template,
        UserState,
    >,
    design_user_state: UserState,
    diag_console: String,
	// Time series keyed by pin name, such as "D0".
    diag_series: HashMap<String, Vec<[f64;2]>>,
    // Latest `/pins` sample keyed by pin name.
    diag_last_pins: Arc<Mutex<Option<HashMap<String, f64>>>>,
    diag_poll_delay: f64,
	next_poll_ms: f64,
}

impl AluminaApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {		
		// Register URI and byte loaders used by device images.
        egui_extras::install_image_loaders(&cc.egui_ctx);
		
        let mut entry =
            ModelEntry::new("icosahedron", Mesh::<()>::icosahedron(100.0, None).float());
        entry.refresh();

        // At an eye distance of 3r, zoom 3*tan(30°) fits the work envelope.
        // Use 1.75 to retain a small margin around the default front view.
        let front_rot = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), -FRAC_PI_2);
        let initial_zoom = 1.75_f32;

        Self {
            rotation: front_rot,
            translation: egui::Vec2::new(0.0, -250.0),
            zoom: initial_zoom,
            models: vec![entry],
            selected_model: Some(0),
            workpiece_data: Arc::new(Mutex::new(None)),
            model_data: Arc::new(Mutex::new(None)),
            wireframe: true,
            edges: true,
            faces: true,
            normals: true,
            vertices: true,
            workarea: true,
            work_size: Vector3::new(200.0, 200.0, 200.0),
            layer_height: 0.20,
            current_layer: 0,
            show_slice: false,
            sliced_layer: None,
            gpu: None,
            gpu_faces: None,
            vertex_storage: Vec::new(),
            selected_tab: Tab::Control,
            diag_poll: false,
            diag_led: false,
            diag_d0:false,diag_d1:false,diag_d2:false,diag_d3:false,
			diag_d4:false,diag_d5:false,diag_d6:false,diag_d7:false,
			diag_d9:false,diag_d11:false,diag_d12:false,diag_d13:false,
			device_info_slot: Arc::new(Mutex::new(None)),
			device_info_requested: false,
            selected_tool: Tool::Laser,
            kerf: 0.1,
            touch_off: true,
            perimeters: 2,
            infill_type: InfillType::Linear,
            endmill_width: 10.0,
            endmill_length: 60.0,
            drill_width: 10.0,
            drill_length: 60.0,
            pixels_wide: 2048,
            pixels_tall: 1024,
            layer_delay: 2.0,
            peel_distance: 15.0,
            design_state: GraphEditorState::default(),
            design_user_state: UserState::default(),
            diag_console: String::new(),
			diag_series: HashMap::new(),
			diag_last_pins: Arc::new(Mutex::new(None)),
			diag_poll_delay: 1.0,
			next_poll_ms: 0.0,
        }
    }
    
    /// Ensure `selected_model` is within bounds or `None` if there are no models.
    fn clamp_selection(&mut self) {
        if self.models.is_empty() {
            self.selected_model = None;
        } else if let Some(i) = self.selected_model {
            if i >= self.models.len() {
                self.selected_model = Some(self.models.len() - 1);
            }
        }
    }

    /// Refresh every model whose instance transform changed.
    fn refresh_models(&mut self) {
        for m in &mut self.models {
            m.refresh();
        }
    }

    /// Rebuild `sliced_layer` for the current Z level.
    fn refresh_slice(&mut self) {
        if !self.show_slice {
            return;
        }

        // Slice the union so overlapping models produce one layer outline.
        let z = self.current_layer as f32 * self.layer_height;
        let plane = csgrs::mesh::plane::Plane::from_normal(Vector3::z(), z.into());
        let mut iter = self.models.iter();
        if let Some(first) = iter.next() {
            let mut combined = first.mesh.clone();
            for m in iter {
                combined = combined.union(&m.mesh);
            }
            self.sliced_layer = Some(combined.slice(plane));
        }
    }

    /// Mark the selected model for rebuilding on the next refresh.
    fn invalidate_selected_model(&mut self) {
        if let Some(m) = self.sel_mut() {
            m.applied_scale = INVALID_SCALE;
            m.applied_offset = Vector3::repeat(f32::NAN);
        }
    }

    /// Replace the selected model's imported geometry.
    fn set_selected_base(&mut self, mesh: Mesh<()>, name: String) {
        if let Some(m) = self.sel_mut() {
            m.base = mesh;
            m.name = name;
            self.invalidate_selected_model();
            self.refresh_models();
            self.refresh_slice();
        }
    }

    /// Return the selected model for mutation.
    fn sel_mut(&mut self) -> Option<&mut ModelEntry> {
        self.selected_model
            .and_then(move |i| self.models.get_mut(i))
    }

    /// Add a model and select it.
    fn add_model(&mut self, mesh: Mesh<()>, name: String) {
        let mut e = ModelEntry::new(name, mesh);
        e.refresh();
        self.models.push(e);
        self.selected_model = Some(self.models.len() - 1);
        self.refresh_slice();
    }
    
    fn diag_log(&mut self, line: impl Into<String>) {
        if !self.diag_console.is_empty() { self.diag_console.push('\n'); }
        self.diag_console.push_str(&line.into());
    }
    fn diag_push_point_named(&mut self, name: &str, x: f64, y: f64) {
        self.diag_series.entry(name.to_string()).or_default().push([x, y]);
    }

    /// Fetch `/pins` once and store its numeric JSON fields in `target`.
    fn poll_pins_once(target: Arc<Mutex<Option<HashMap<String, f64>>>>) {
        execute(async move {
            match http_get_text("/pins").await {
                Ok(body) => {
                    // Firmware may encode states as either JSON integers or floats.
                    let parsed: Result<HashMap<String, f64>, _> = serde_json::from_str::<HashMap<String, serde_json::Value>>(&body)
                        .map(|m| {
                            m.into_iter()
                                .filter_map(|(k, v)| v.as_f64().or_else(|| v.as_u64().map(|u| u as f64)).map(|f| (k, f)))
                                .collect()
                        });
                    match parsed {
                        Ok(map) => { *target.lock().unwrap() = Some(map); }
                        Err(e) => log::error!("parse /pins failed: {:?}", e),
                    }
                }
                Err(e) => log::error!("poll /pins failed: {:?}", e),
            }
        });
    }
    
    fn is_pin_checked(&self, name: &str) -> bool {
        match name {
            "D0"  => self.diag_d0,
            "D1"  => self.diag_d1,
            "D2"  => self.diag_d2,
            "D3"  => self.diag_d3,
            "D4"  => self.diag_d4,
            "D5"  => self.diag_d5,
            "D6"  => self.diag_d6,
            "D7"  => self.diag_d7,
            "D9"  => self.diag_d9,
            "D11" => self.diag_d11,
            "D12" => self.diag_d12,
            "D13" => self.diag_d13,
            _ => false,
        }
	}
}

impl AluminaApp {
    /// Rebuild and upload the line and face buffers for the current scene.
    unsafe fn sync_buffers(&mut self, gl: &glow::Context) {
        self.vertex_storage.clear();
        let mut faces: Vec<f32> = Vec::new();

        // Draw a 10 mm XY grid and the remaining work-envelope edges.
        if self.workarea {
            let minor = [0.55, 0.55, 0.55];
            let major = [1.0, 1.0, 1.0];

            let hx = self.work_size.x * 0.5;
            let hy = self.work_size.y * 0.5;
            let hz = self.work_size.z;

            // Lines at constant X.
            for i in 0..=(self.work_size.x / 10.0) as i32 {
                let x = -hx + i as f32 * 10.0;
                let col = if i % 10 == 0 { major } else { minor };
                self.vertex_storage.extend_from_slice(&[
                    x, -hy, 0.0, col[0], col[1], col[2], x, hy, 0.0, col[0], col[1], col[2],
                ]);
            }

            // Lines at constant Y.
            for i in 0..=(self.work_size.y / 10.0) as i32 {
                let y = -hy + i as f32 * 10.0;
                let col = if i % 10 == 0 { major } else { minor };
                self.vertex_storage.extend_from_slice(&[
                    -hx, y, 0.0, col[0], col[1], col[2], hx, y, 0.0, col[0], col[1], col[2],
                ]);
            }

            let edge = major;

            // Four vertical edges.
            for (sx, sy) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
                let x = sx * hx;
                let y = sy * hy;
                self.vertex_storage.extend_from_slice(&[
                    x, y, 0.0, edge[0], edge[1], edge[2], // bottom
                    x, y, hz, edge[0], edge[1], edge[2], // top
                ]);
            }

            // Top rectangle at Z = hz.
            self.vertex_storage.extend_from_slice(&[
                -hx, -hy, hz, edge[0], edge[1], edge[2], hx, -hy, hz, edge[0], edge[1], edge[2],
                hx, -hy, hz, edge[0], edge[1], edge[2], hx, hy, hz, edge[0], edge[1], edge[2], hx,
                hy, hz, edge[0], edge[1], edge[2], -hx, hy, hz, edge[0], edge[1], edge[2], -hx, hy,
                hz, edge[0], edge[1], edge[2], -hx, -hy, hz, edge[0], edge[1], edge[2],
            ]);
        }

        // Draw either the active slice or the model inspection overlays.
        fn add_line_string(ls: &LineString<f64>, z: f32, col: [f32; 3], out: &mut Vec<f32>) {
            for w in ls.0.windows(2) {
                let a = w[0];
                let b = w[1];
                out.extend_from_slice(&[
                    a.x as f32, a.y as f32, z, col[0], col[1], col[2], b.x as f32, b.y as f32, z,
                    col[0], col[1], col[2],
                ]);
            }
        }

        if self.show_slice {
            const PURPLE: [f32; 3] = [0.6, 0.1, 0.8];
            if let Some(slice) = &self.sliced_layer {
                let z = self.current_layer as f32 * self.layer_height;

                for geom in &slice.geometry.0 {
                    match geom {
                        Geometry::LineString(ls) => {
                            add_line_string(ls, z, PURPLE, &mut self.vertex_storage)
                        }
                        Geometry::Polygon(poly) => {
                            add_line_string(&poly.exterior(), z, PURPLE, &mut self.vertex_storage);
                            for inner in poly.interiors() {
                                add_line_string(inner, z, PURPLE, &mut self.vertex_storage);
                            }
                        }
                        _ => {}
                    }
                }
            }
        } else {
            // Model wireframe.
            if self.edges {
                const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
                for model_entry in &self.models {
                    let model = &model_entry.mesh;
                    for p in &model.polygons {
                        for (a, b) in p.edges() {
                            self.vertex_storage.extend_from_slice(&[
                                a.pos.x as f32,
                                a.pos.y as f32,
                                a.pos.z as f32,
                                WHITE[0],
                                WHITE[1],
                                WHITE[2],
                                b.pos.x as f32,
                                b.pos.y as f32,
                                b.pos.z as f32,
                                WHITE[0],
                                WHITE[1],
                                WHITE[2],
                            ]);
                        }
                    }
                }
            }

            // Filled model faces.
            if self.faces {
                for model_entry in &self.models {
                    let model = &model_entry.mesh;
                    for p in &model.polygons {
                        let verts = &p.vertices;
                        if verts.len() >= 3 {
                            for i in 1..verts.len() - 1 {
                                for v in [&verts[0].pos, &verts[i].pos, &verts[i + 1].pos] {
                                    faces.extend_from_slice(&[
                                        v.x as f32,
                                        v.y as f32,
                                        v.z as f32,
                                        renderer::EGUI_BLUE[0],
                                        renderer::EGUI_BLUE[1],
                                        renderer::EGUI_BLUE[2],
                                    ]);
                                }
                            }
                        }
                    }
                }
            }

            // Polygon-normal indicators.
            if self.normals {
                const NORMAL_COL: [f32; 3] = [1.0, 0.0, 0.0]; // red
                let normal_len = (self.work_size.norm() * 0.04) as f32; // ≈ 4 % of diag

                for model_entry in &self.models {
                    let model = &model_entry.mesh;
                    for p in &model.polygons {
                        // Anchor each normal at its polygon centroid.
                        let mut c = Vector3::zeros();
                        for v in &p.vertices {
                            c += Vector3::new(v.pos.x as f32, v.pos.y as f32, v.pos.z as f32);
                        }
                        c /= p.vertices.len() as f32;

                        let n = Vector3::<f32>::new(
                            p.plane.normal().x as f32,
                            p.plane.normal().y as f32,
                            p.plane.normal().z as f32,
                        )
                        .normalize()
                            * normal_len;

                        self.vertex_storage.extend_from_slice(&[
                            c.x,
                            c.y,
                            c.z,
                            NORMAL_COL[0],
                            NORMAL_COL[1],
                            NORMAL_COL[2],
                            c.x + n.x,
                            c.y + n.y,
                            c.z + n.z,
                            NORMAL_COL[0],
                            NORMAL_COL[1],
                            NORMAL_COL[2],
                        ]);
                    }
                }
            }

            // Vertex indicators.
            if self.vertices {
                const VERT_COL: [f32; 3] = [1.0, 1.0, 0.0]; // yellow
                let r = (self.work_size.norm() * 0.005) as f32; // ≈ 0.5 % of diag

                // Quantize positions to avoid drawing duplicate indicators.
                let mut seen: HashSet<(i64, i64, i64)> = HashSet::new();
                let quant = 1_000_000.0; // 1 µm grid

                for model_entry in &self.models {
                    let model = &model_entry.mesh;
                    for p in &model.polygons {
                        for v in &p.vertices {
                            let key = (
                                (v.pos.x * quant) as i64,
                                (v.pos.y * quant) as i64,
                                (v.pos.z * quant) as i64,
                            );
                            if seen.insert(key) {
                                let c =
                                    Vector3::new(v.pos.x as f32, v.pos.y as f32, v.pos.z as f32);
                                add_vertex_sphere(c, r, VERT_COL, &mut faces);
                            }
                        }
                    }
                }
            }
        }

        // Upload the rebuilt buffers.
        if let Some(lines_gpu) = &self.gpu {
            if let Ok(mut g) = lines_gpu.lock() {
                unsafe { g.upload_vertices(gl, &self.vertex_storage) };
            }
        }

        // Release the optional triangle buffer when it is empty.
        let need_tris = !faces.is_empty();
        if need_tris {
            let faces_gpu = self.gpu_faces.get_or_insert_with(|| {
                Arc::new(Mutex::new(unsafe { renderer::GpuLines::new(gl) }))
            });
            if let Ok(mut g) = faces_gpu.lock() {
                unsafe { g.upload_vertices(gl, &faces) };
            }
        } else {
            self.gpu_faces = None;
        }
    }
}

impl eframe::App for AluminaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("tab_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.selected_tab, Tab::Diagnostics, "Diagnostics");
                ui.selectable_value(&mut self.selected_tab, Tab::Design, "Design");
                ui.selectable_value(&mut self.selected_tab, Tab::Control, "Control");
            });
        });

        match self.selected_tab {
            Tab::Control => {
                egui::SidePanel::left("side_panel")
                    .resizable(false)
                    .min_width(140.0)
                    .show(ctx, |ui| {
                        ui.heading("Control");
                        ui.separator();

                        ui.label("Loaded models");
                        let mut remove: Option<usize> = None;
                        for (i, m) in self.models.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                if ui
                                    .selectable_label(self.selected_model == Some(i), &m.name)
                                    .clicked()
                                {
                                    self.selected_model = Some(i);
                                }
                                if ui.button("x").clicked() {
                                    remove = Some(i);
                                }
                            });
                        }
                        if ui.button("Add…").clicked() {
                            self.selected_model = None; // -> add after file dialog
                            spawn_file_picker(
                                Arc::clone(&self.model_data),
                                "Model mesh (stl,dxf)",
                                &["stl", "dxf", "obj", "ply", "amf"],
                            );
                        }
                        if let Some(idx) = remove {
                            self.models.remove(idx);
                            self.clamp_selection();
                        }

                        ui.separator();
                        ui.label("Snap view");
                        ui.horizontal_wrapped(|ui| {
                            let pitch =
                                UnitQuaternion::from_axis_angle(&Vector3::x_axis(), -FRAC_PI_2); //  -90° about X  (Z-up ➜ Y-up)
                            if ui.button("Front").clicked() {
                                self.rotation = pitch;
                            } //  -90° about X
                            if ui.button("Back").clicked() {
                                self.rotation =
                                    pitch * UnitQuaternion::from_axis_angle(&Vector3::z_axis(), PI);
                            } // 180° roll
                            if ui.button("Left").clicked() {
                                self.rotation =
                                    UnitQuaternion::from_axis_angle(&Vector3::y_axis(), FRAC_PI_2)
                                        * pitch;
                            } // +90° yaw
                            if ui.button("Right").clicked() {
                                self.rotation =
                                    UnitQuaternion::from_axis_angle(&Vector3::y_axis(), -FRAC_PI_2)
                                        * pitch;
                            } // –90° yaw
                            if ui.button("Top").clicked() {
                                self.rotation = UnitQuaternion::identity();
                            } // no change
                            if ui.button("Bottom").clicked() {
                                self.rotation =
                                    UnitQuaternion::from_axis_angle(&Vector3::x_axis(), PI);
                            } // look from below
                        });

                        ui.separator();
                        ui.checkbox(&mut self.edges, "edges");
                        ui.checkbox(&mut self.faces, "faces");
                        ui.checkbox(&mut self.normals, "normals");
                        ui.checkbox(&mut self.vertices, "vertices");
                        ui.checkbox(&mut self.workarea, "Work area");

                        ui.separator();
                        ui.collapsing("Model scale", |ui| {
                            if let Some(m) = self.sel_mut() {
                                let mut changed = false;

                                ui.horizontal(|ui| {
                                    ui.label("X:");
                                    changed |= ui
                                        .add(
                                            egui::DragValue::new(&mut m.scale.x)
                                                .speed(0.01)
                                                .range(0.01..=100.0),
                                        )
                                        .changed();
                                });
                                ui.horizontal(|ui| {
                                    ui.label("Y:");
                                    changed |= ui
                                        .add(
                                            egui::DragValue::new(&mut m.scale.y)
                                                .speed(0.01)
                                                .range(0.01..=100.0),
                                        )
                                        .changed();
                                });
                                ui.horizontal(|ui| {
                                    ui.label("Z:");
                                    changed |= ui
                                        .add(
                                            egui::DragValue::new(&mut m.scale.z)
                                                .speed(0.01)
                                                .range(0.01..=100.0),
                                        )
                                        .changed();
                                });

                                if ui.button("Reset scale").clicked() {
                                    m.scale = Vector3::new(1.0, 1.0, 1.0);
                                    changed = true;
                                }

                                // Invalidate within the existing mutable borrow.
                                if changed {
                                    m.applied_scale = INVALID_SCALE;
                                }
                            } else {
                                ui.label("No model selected");
                            }
                        });

                        ui.separator();
                        ui.collapsing("Model position", |ui| {
                            if let Some(m) = self.sel_mut() {
                                let mut changed = false;

                                if ui.button("Float (Z = 0)").clicked() {
                                    m.offset = Vector3::zeros();
                                    m.base = m.base.clone().float();
                                    changed = true;
                                }
                                if ui.button("Center").clicked() {
                                    m.offset = Vector3::zeros();
                                    m.base = m.base.clone().center();
                                    changed = true;
                                }

                                ui.horizontal(|ui| {
                                    ui.label("X:");
                                    changed |= ui
                                        .add(egui::DragValue::new(&mut m.offset.x).speed(1.0))
                                        .changed();
                                });
                                ui.horizontal(|ui| {
                                    ui.label("Y:");
                                    changed |= ui
                                        .add(egui::DragValue::new(&mut m.offset.y).speed(1.0))
                                        .changed();
                                });
                                ui.horizontal(|ui| {
                                    ui.label("Z:");
                                    changed |= ui
                                        .add(egui::DragValue::new(&mut m.offset.z).speed(1.0))
                                        .changed();
                                });

                                if ui.button("Reset position").clicked() {
                                    m.offset = Vector3::zeros();
                                    changed = true;
                                }

                                if changed {
                                    // Invalidate without taking a second mutable borrow.
                                    m.applied_offset = Vector3::repeat(f32::NAN);
                                }
                            } else {
                                ui.label("No model selected");
                            }
                        });

                        ui.separator();
                        ui.collapsing("Work area (mm)", |ui| {
                            ui.horizontal(|ui| {
                                ui.label("X:");
                                ui.add(egui::DragValue::new(&mut self.work_size.x).speed(1.0));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Y:");
                                ui.add(egui::DragValue::new(&mut self.work_size.y).speed(1.0));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Z:");
                                ui.add(egui::DragValue::new(&mut self.work_size.z).speed(1.0));
                            });
                        });

                        ui.separator();
                        ui.collapsing("Tool settings", |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Tool:");
                                egui::ComboBox::from_id_salt("tool_select")
                                    .selected_text(self.selected_tool.to_string())
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.selected_tool,
                                            Tool::Laser,
                                            "Laser",
                                        );
                                        ui.selectable_value(
                                            &mut self.selected_tool,
                                            Tool::Plasma,
                                            "Plasma",
                                        );
                                        ui.selectable_value(
                                            &mut self.selected_tool,
                                            Tool::Extruder,
                                            "Extruder",
                                        );
                                        ui.selectable_value(
                                            &mut self.selected_tool,
                                            Tool::Endmill,
                                            "Endmill",
                                        );
                                        ui.selectable_value(
                                            &mut self.selected_tool,
                                            Tool::Drill,
                                            "Drill",
                                        );
                                        ui.selectable_value(
                                            &mut self.selected_tool,
                                            Tool::DlpLcd,
                                            "DLP / LCD",
                                        );
                                    });
                            });

                            match self.selected_tool {
                                Tool::Laser => {
                                    ui.horizontal(|ui| {
                                        ui.label("Kerf (mm):");
                                        ui.add(
                                            egui::DragValue::new(&mut self.kerf)
                                                .speed(0.01)
                                                .range(0.0..=5.0),
                                        );
                                    });
                                }
                                Tool::Plasma => {
                                    ui.checkbox(&mut self.touch_off, "Touch off");
                                }
                                Tool::Extruder => {
                                    ui.horizontal(|ui| {
                                        ui.label("Perimeters:");
                                        ui.add(
                                            egui::DragValue::new(&mut self.perimeters)
                                                .speed(1)
                                                .range(0..=10),
                                        );
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("Infill type:");
                                        egui::ComboBox::from_id_salt("infill_type")
                                            .selected_text(self.infill_type.to_string())
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(
                                                    &mut self.infill_type,
                                                    InfillType::Linear,
                                                    "Linear",
                                                );
                                                ui.selectable_value(
                                                    &mut self.infill_type,
                                                    InfillType::Gyroid,
                                                    "Gyroid",
                                                );
                                                ui.selectable_value(
                                                    &mut self.infill_type,
                                                    InfillType::SchwarzP,
                                                    "Schwarz P",
                                                );
                                                ui.selectable_value(
                                                    &mut self.infill_type,
                                                    InfillType::SchwarzD,
                                                    "Schwarz D",
                                                );
                                            });
                                    });
                                }
                                Tool::Endmill => {
                                    ui.horizontal(|ui| {
                                        ui.label("Endmill width (mm):");
                                        ui.add(
                                            egui::DragValue::new(&mut self.endmill_width)
                                                .speed(0.1)
                                                .range(0.1..=100.0),
                                        );
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("Endmill length (mm):");
                                        ui.add(
                                            egui::DragValue::new(&mut self.endmill_length)
                                                .speed(0.1)
                                                .range(1.0..=300.0),
                                        );
                                    });
                                }
                                Tool::Drill => {
                                    ui.horizontal(|ui| {
                                        ui.label("Drill width (mm):");
                                        ui.add(
                                            egui::DragValue::new(&mut self.drill_width)
                                                .speed(0.1)
                                                .range(0.1..=100.0),
                                        );
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("Drill length (mm):");
                                        ui.add(
                                            egui::DragValue::new(&mut self.drill_length)
                                                .speed(0.1)
                                                .range(1.0..=300.0),
                                        );
                                    });
                                }
                                Tool::DlpLcd => {
                                    ui.horizontal(|ui| {
                                        ui.label("Pixels wide:");
                                        ui.add(
                                            egui::DragValue::new(&mut self.pixels_wide)
                                                .speed(1)
                                                .range(1..=8192),
                                        );
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("Pixels tall:");
                                        ui.add(
                                            egui::DragValue::new(&mut self.pixels_tall)
                                                .speed(1)
                                                .range(1..=8192),
                                        );
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("Layer delay (s):");
                                        ui.add(
                                            egui::DragValue::new(&mut self.layer_delay)
                                                .speed(0.1)
                                                .range(0.0..=60.0),
                                        );
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("Peel distance (mm):");
                                        ui.add(
                                            egui::DragValue::new(&mut self.peel_distance)
                                                .speed(0.1)
                                                .range(0.0..=100.0),
                                        );
                                    });
                                }
                            }
                        });

                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label("Layer height (mm):");
                            ui.add(
                                egui::DragValue::new(&mut self.layer_height)
                                    .speed(0.01)
                                    .range(0.01..=10.0),
                            );
                        });

                        ui.horizontal(|ui| {
                            let max_layers = (self.work_size.z / self.layer_height).floor() as i32;
                            let prev = self.current_layer;
                            ui.label("Current layer:");
                            ui.add(
                                egui::DragValue::new(&mut self.current_layer)
                                    .range(0..=max_layers)
                                    .speed(1),
                            );
                            if self.current_layer != prev {
                                self.refresh_slice();
                            }
                        });
                        if ui.checkbox(&mut self.show_slice, "slice").changed() {
                            self.refresh_slice();
                        }

                        ui.separator();
                        if ui.button("load workpiece").clicked() {
                            spawn_file_picker(
                                Arc::clone(&self.workpiece_data),
                                "Workpiece mesh (stl,dxf)",
                                &["stl", "dxf"],
                            );
                        }
                        if ui.button("send").clicked(){
							// `g0` is the firmware's current send command.
							send_queue_command("g0");
						}
                        if ui.button("toggle").clicked() {
                            self.wireframe = !self.wireframe;
                        }
                    });

                let workpiece_bytes_opt = {
                    let mut guard = self.workpiece_data.lock().unwrap();
                    guard.take()
                };
                if let Some(bytes) = workpiece_bytes_opt {
                    if let Some(mesh) = load_mesh_from_bytes(&bytes) {
                        self.add_model(mesh.float(), "workpiece".into());
                        log::info!("[alumina] workpiece loaded ({} bytes)", bytes.len());
                    } else {
                        log::error!("Could not parse workpiece file");
                    }
                }

                let model_bytes_opt = {
                    let mut guard = self.model_data.lock().unwrap();
                    guard.take()
                };
                if let Some(bytes) = model_bytes_opt {
                    if let Some(mesh) = load_mesh_from_bytes(&bytes) {
                        let name = "model".to_string();
                        // Replace the selection, or add a model after an Add action.
                        if self.selected_model.is_some() {
                            self.set_selected_base(mesh.float(), name);
                        } else {
                            self.add_model(mesh.float(), name);
                        }
                        log::info!("[alumina] model loaded ({} bytes)", bytes.len());
                    } else {
                        log::error!("Could not parse model file – unsupported or corrupt");
                    }
                }

                self.refresh_models();
                self.refresh_slice();

                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_min_size(ui.available_size());
                    let (rect, response) =
                        ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());

                    if response.dragged() {
                        let delta = response.drag_delta();
                        let input = ui.input(|i| i.clone());
                        if input.pointer.primary_down() {
                            // Primary drag rotates the model.
                            let yaw = delta.x * 0.01;
                            let pitch = delta.y * 0.01;
                            self.rotation =
                                UnitQuaternion::from_axis_angle(&Vector3::y_axis(), yaw)
                                    * UnitQuaternion::from_axis_angle(&Vector3::x_axis(), pitch)
                                    * self.rotation;
                        } else if input.pointer.middle_down() {
                            // Middle drag pans the model.
                            self.translation += -delta;
                        }
                    }
                    
					// Egui reports pinch gestures as a multiplicative zoom delta.
					let pinch = ui.input(|i| i.zoom_delta());
					if (pinch - 1.0).abs() > f32::EPSILON {
						self.zoom = (self.zoom / pinch).clamp(0.1, 500.0);
					}

                    // Vertical scrolling changes camera distance.
                    let scroll = ui.input(|i| i.raw_scroll_delta.y);
                    if scroll.abs() > 0.0 {
                        self.zoom = (self.zoom * (1.0 + scroll * 0.001)).clamp(0.0, 500.0);
                    }

                    if let Some(gl) = _frame.gl() {
                        if self.gpu.is_none() {
                            self.gpu =
                                Some(Arc::new(Mutex::new(unsafe { renderer::GpuLines::new(gl) })));
                        }

                        unsafe { self.sync_buffers(gl) };

                        // Schedule GL painting after egui's meshes.
                        if let Some(lines_gpu) = &self.gpu {
                            let lines_gpu = lines_gpu.clone();
                            let faces_gpu = self.gpu_faces.clone();
                            let mvp = mvp(self, rect); // copy for the closure

                            let callback = egui_glow::CallbackFn::new(move |_info, painter| {
                                let gl = painter.gl();
                                unsafe {
                                    gl.enable(glow::DEPTH_TEST);
                                    gl.depth_func(glow::LEQUAL);
                                    gl.clear(glow::DEPTH_BUFFER_BIT);

                                    // Offset filled faces to keep coplanar outlines crisp.
                                    if let Some(faces_gpu) = &faces_gpu {
                                        if let Ok(f) = faces_gpu.lock() {
                                            gl.enable(glow::POLYGON_OFFSET_FILL);
                                            gl.polygon_offset(1.0, 1.0);
                                            f.paint_tris(gl, mvp);
                                            gl.disable(glow::POLYGON_OFFSET_FILL);
                                        }
                                    }
                                    if let Ok(l) = lines_gpu.lock() {
                                        l.paint(gl, mvp);
                                    }
                                }
                            });

                            ui.painter().add(egui::PaintCallback {
                                rect,
                                callback: Arc::new(callback),
                            });
                        }
                    }
                });
            }

            Tab::Diagnostics => {
                egui::SidePanel::left("diag_side")
                    .resizable(false)
                    .min_width(140.0)
                    .show(ctx, |ui| {
                        ui.heading("Diagnostics");
                        ui.separator();
                        ui.horizontal(|ui| {
							if ui.button("Scan Wi-Fi").clicked() {
								send_queue_command("scan_wifi");
							}
							if ui.button("Set Wi-Fi").clicked() {
								// The current firmware records this request for later handling.
								send_queue_command("set_wifi");
							}
						});
                        ui.separator();
                        ui.horizontal(|ui| {
							if ui.checkbox(&mut self.diag_poll, "Poll").changed() {
								if self.diag_poll {
									Self::poll_pins_once(Arc::clone(&self.diag_last_pins));
									if let Some(perf) = web_sys::window().and_then(|w| w.performance()) {
										let now = perf.now();
										self.next_poll_ms = now + self.diag_poll_delay * 1000.0;
									} else {
										self.next_poll_ms = 0.0;
									}
									ctx.request_repaint(); // ensure an immediate frame to show the first result
								}
							}
							ui.label("every");
							ui.add(
								egui::DragValue::new(&mut self.diag_poll_delay)
									.speed(0.1)
									.range(0.2..=5.0)   // 0.2s .. 5.0s
									.suffix(" s"),
							);
						});
						if ui.checkbox(&mut self.diag_led,"Status LED").changed(){
							if self.diag_led { send_queue_command("status_on"); }
							else { send_queue_command("status_off"); }
						}
						ui.separator();
						ui.label("GPIO pins");
						let gpio_row = |label: &str,
											state: &mut bool,
											high: &'static str,
											low:  &'static str,
											ui: &mut egui::Ui| {
							if ui.checkbox(state, label).changed() {
								if *state { send_queue_command(high); }
								else      { send_queue_command(low);  }
							}
						};
						gpio_row("D0",&mut self.diag_d0,"d0_high","d0_low",ui);
						gpio_row("D1",&mut self.diag_d1,"d1_high","d1_low",ui);
						gpio_row("D2",&mut self.diag_d2,"d2_high","d2_low",ui);
						gpio_row("D3",&mut self.diag_d3,"d3_high","d3_low",ui);
						gpio_row("D4",&mut self.diag_d4,"d4_high","d4_low",ui);
						gpio_row("D5",&mut self.diag_d5,"d5_high","d5_low",ui);
						gpio_row("D6",&mut self.diag_d6,"d6_high","d6_low",ui);
						gpio_row("D7",&mut self.diag_d7,"d7_high","d7_low",ui);
						gpio_row("D9",&mut self.diag_d9,"d9_high","d9_low",ui);
						gpio_row("D11",&mut self.diag_d11,"d11_high","d11_low",ui);
						gpio_row("D12",&mut self.diag_d12,"d12_high","d12_low",ui);
						gpio_row("D13",&mut self.diag_d13,"d13_high","d13_low",ui);
					});

				egui::CentralPanel::default().show(ctx, |ui| {
					// Keep producing frames while diagnostics polling is active.
					if self.diag_poll {
						ctx.request_repaint_after(std::time::Duration::from_secs_f64(self.diag_poll_delay));
					}

					if self.diag_poll {
						if let Some(perf) = web_sys::window().and_then(|w| w.performance()) {
							let now = perf.now();
							if now >= self.next_poll_ms {
								// Advance by whole periods to avoid drift after a late frame.
								let period = self.diag_poll_delay * 1000.0;
								while self.next_poll_ms <= now {
									self.next_poll_ms += period;
								}
								Self::poll_pins_once(Arc::clone(&self.diag_last_pins));
							}
						}
					}

					// Add the latest sample to selected pin series and the console.
					if let Some(pins) = { let mut g = self.diag_last_pins.lock().unwrap(); g.take() } {
						let t = (web_sys::window().and_then(|w| w.performance()).map(|p| p.now()).unwrap_or(0.0)) / 1000.0;
						let mut line = format!("t={:.02}s ", t);
						for (name, val) in pins.iter() {
							if self.is_pin_checked(name) {
								self.diag_push_point_named(name, t as f64, *val);
								line.push_str(&format!(" {}={}", name, *val as i32));
							}
						}
						if line.trim() != "t=0.00s" {
							self.diag_log(line);
						}
						ctx.request_repaint();
					}
					
					// Split diagnostics into equal status and console regions.
					let total = ui.available_size();
					let half_h = total.y / 2.0;
					
					// Fetch device metadata once on the first diagnostics frame.
					if !self.device_info_requested {
						self.device_info_requested = true;
						fetch_board_info(Arc::clone(&self.device_info_slot));
					}
					
					// Device images may be returned as same-origin relative URLs.
					fn absolutize_url(p: &str) -> String {
						if p.starts_with("http://") || p.starts_with("https://") { return p.to_owned(); }
						let win = web_sys::window().expect("no window");
						let origin = win.location().origin().unwrap_or_else(|_| "".into());
						format!("{origin}{p}")
					}
					
					// Read device metadata without blocking the UI.
					let fetched = { let g = self.device_info_slot.lock().unwrap(); g.clone() };
					let mut device_img_url = String::from("/device/image");
					let mut device_display_name = String::from("Device");
					if let Some((_name, display_name, _image_mime, image_url)) = fetched {
						device_display_name = display_name;
						device_img_url = image_url;
					}

					let device_img_url = absolutize_url(&device_img_url);
					log::warn!("device image uri = {}", device_img_url);

					ui.allocate_ui(egui::vec2(total.x, half_h), |ui| {
						ui.columns(2, |cols| {
							cols[0].heading("IO Status");
							cols[0].add_space(4.0);
							egui_plot::Plot::new("diag_plot")
								.width(cols[0].available_width())
								.height(cols[0].available_height())
								.show(&mut cols[0], |plot_ui| {
									for (name, series) in &self.diag_series {
										if self.is_pin_checked(name) && !series.is_empty() {
											let points = egui_plot::PlotPoints::from(series.clone());
											plot_ui.line(egui_plot::Line::new(points).name(name.clone()));
										}
									}
								});

							cols[1].heading(device_display_name);
							cols[1].add_space(4.0);
							let max = cols[1].available_size();
							cols[1].add(
								egui::Image::from_uri(device_img_url.clone())
									.max_size(max) // use the column's size instead of hard 400px
							);
						});
					});

					ui.allocate_ui(egui::vec2(total.x, half_h), |ui| {
						ui.horizontal(|ui| {
							ui.heading("Console");
							if ui.button("Clear").clicked() {
								self.diag_console.clear();
								self.diag_series.clear();
							}
							if ui.button("Refresh queue").clicked() {
								execute(async {
									match http_get_text("/queue").await {
										Ok(s) => log::info!("/queue: {}", s),
										Err(e) => log::error!("GET /queue failed: {:?}", e),
									}
								});
							}
						});

						egui::ScrollArea::vertical()
							.stick_to_bottom(true)
							.show(ui, |ui| {
								let te = egui::TextEdit::multiline(&mut self.diag_console)
									.desired_width(f32::INFINITY)
									.interactive(false);
								ui.add_sized(ui.available_size(), te);
							});
					});
				});
            }

            Tab::Design => {
                egui::SidePanel::left("design_side")
                    .resizable(false)
                    .min_width(140.0)
                    .show(ctx, |ui| {
                        ui.heading("Design");
                        ui.separator();
                        if ui.button("Clear graph").clicked() {
                            self.design_state = GraphEditorState::default();
                        }
                        if ui.button("Apply to model").clicked() {
                            let roots = design_graph::graph_roots(&self.design_state.graph);
                            log::warn!("roots: {:#?}", roots);
                            if roots.is_empty() {
                                log::warn!("Apply to model: No root nodes found in the graph.");
                            }
                            for root_out in roots {
                                match design_graph::evaluate(&self.design_state.graph, root_out) {
                                    Ok(mesh) => self.add_model(mesh.float(), "graph".into()),
                                    Err(e) => log::error!(
                                        "Graph eval failed for root {:?}: {e}",
                                        root_out
                                    ),
                                }
                            }
                        }
                        if ui.button("Save .graph").clicked() {
                            // Graph serialization is not implemented yet.
                        }
                    });

                egui::CentralPanel::default().show(ctx, |ui| {
					ui.set_min_size(ui.available_size());

					// Preserve emptiness before drawing so the editor can show a hint.
					let graph_is_empty =
						self.design_state.graph.inputs.is_empty() && self.design_state.graph.outputs.is_empty();

					let resp = self.design_state.draw_graph_editor(
						ui,
						AllTemplates,
						&mut self.design_user_state,
						Vec::<egui_node_graph2::NodeResponse<
							design_graph::EmptyUserResponse,
							design_graph::NodeData,
						>>::new(),
					);
					_ = resp;

					// Explain how to create the first node.
					if graph_is_empty {
						let rect = ui.max_rect();
						let painter = ui.painter();
						painter.text(
							rect.center(),
							egui::Align2::CENTER_CENTER,
							"Right-click to open menu",
							egui::FontId::proportional(18.0),
							ui.visuals().weak_text_color(),
						);
					}
				});
            }
        }
    }
}

/// Build a model-view-projection matrix for the current camera and viewport.
///
/// `zoom` is a dolly factor: 1 is the default distance and 2 halves it.
fn mvp(app: &AluminaApp, rect: egui::Rect) -> Matrix4<f32> {
    let radius = app.work_size.norm() * 0.5;
    let base_eye = radius * 3.0;
    let eye = Point3::new(0.0, 0.0, base_eye / app.zoom);

    let aspect = rect.width() / rect.height();
    let proj = Perspective3::new(aspect, 60_f32.to_radians(), 0.1, 10_000.0).to_homogeneous();
    let view = nalgebra::Isometry3::look_at_rh(
        &eye,
        &Point3::origin(),            // target
        &Vector3::new(0.0, 1.0, 0.0), // up
    )
    .to_homogeneous();

    // Convert screen-pixel panning into world coordinates.
    let pixels_per_world = rect.height() / (radius * 2.0);
    let pan = Vector3::new(
        -app.translation.x / pixels_per_world,
        app.translation.y / pixels_per_world,
        0.0,
    );
    let model = Translation3::from(pan).to_homogeneous() * app.rotation.to_homogeneous();

    proj * view * model
}

/// Append an icosahedral vertex marker centered at `c`.
fn add_vertex_sphere(c: Vector3<f32>, r: f32, col: [f32; 3], out: &mut Vec<f32>) {
    // Golden-ratio icosahedron: 12 vertices and 20 triangles.
    const PHI: f32 = 1.618_034;
    const V: &[[f32; 3]] = &[
        [-1.0, PHI, 0.0],
        [1.0, PHI, 0.0],
        [-1.0, -PHI, 0.0],
        [1.0, -PHI, 0.0],
        [0.0, -1.0, PHI],
        [0.0, 1.0, PHI],
        [0.0, -1.0, -PHI],
        [0.0, 1.0, -PHI],
        [PHI, 0.0, -1.0],
        [PHI, 0.0, 1.0],
        [-PHI, 0.0, -1.0],
        [-PHI, 0.0, 1.0],
    ];
    const I: &[[u16; 3]] = &[
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];

    for idx in I {
        for &i in idx {
            let v = Vector3::from(V[i as usize]) * r + c;
            out.extend_from_slice(&[v.x, v.y, v.z, col[0], col[1], col[2]]);
        }
    }
}

fn spawn_file_picker(
    target: Arc<Mutex<Option<Vec<u8>>>>,
    _filter_name: &'static str,
    exts: &'static [&'static str],
) {
    // The browser event loop owns the asynchronous file dialog workflow.
    execute(async move {
        // Create a hidden file input for the requested extensions.
        let document = window()
            .expect("no window")
            .document()
            .expect("no document");
        let input: HtmlInputElement = document
            .create_element("input")
            .unwrap()
            .dyn_into()
            .unwrap();
        input.set_type("file");

        let accept = exts
            .iter()
            .map(|e| format!(".{e}"))
            .collect::<Vec<_>>()
            .join(",");
        input.set_accept(&accept);

        input.style().set_property("display", "none").unwrap();
        document.body().unwrap().append_child(&input).unwrap();

        // Bridge the browser's change event into an awaitable channel.
        let (tx, rx) = oneshot::channel::<()>();

        // The event closure may run repeatedly, but the sender is consumed once.
        let tx_cell = Rc::new(RefCell::new(Some(tx)));
        let tx_handle = Rc::clone(&tx_cell);

        let closure = Closure::<dyn FnMut(Event)>::wrap(Box::new(move |_e| {
            if let Some(sender) = tx_handle.borrow_mut().take() {
                let _ = sender.send(());
            }
        }));
        input
            .add_event_listener_with_callback("change", closure.as_ref().unchecked_ref())
            .unwrap();
        // Keep the callback alive for the dynamically created element.
        closure.forget();

        input.click();
        rx.await.ok();

        // Copy the selected browser file into Rust-owned bytes.
        let files = input.files().unwrap();
        if files.length() == 0 {
            return;
        }
        let file = files.get(0).unwrap();

        let buf_promise = file.array_buffer();
        let js_buf = JsFuture::from(buf_promise).await.unwrap();
        let u8_array = Uint8Array::new(&js_buf);
        let mut bytes = vec![0u8; u8_array.length() as usize];
        u8_array.copy_to(&mut bytes);

        *target.lock().unwrap() = Some(bytes);
    });
}

/// POST a simple text command to the firmware `/queue` endpoint.
fn send_queue_command(cmd:&'static str){
    execute(async move{
        use wasm_bindgen::prelude::*;
        use wasm_bindgen::JsCast;
        use wasm_bindgen_futures::JsFuture;
        use web_sys::{Request,RequestInit,Window,Response};
        let window:Window=web_sys::window().expect("no window");
        let opts=RequestInit::new();
        opts.set_method("POST");
        opts.set_body(&JsValue::from_str(cmd));
        let request=Request::new_with_str_and_init("/queue",&opts).unwrap();
        request.headers().set("Accept","text/plain").ok();
        request.headers().set("Content-Type","text/plain").ok();
        let resp_value=JsFuture::from(window.fetch_with_request(&request)).await;
        if let Ok(val)=resp_value{
            let _resp:Response=val.dyn_into().unwrap();
        }
    });
}
#[derive(serde::Deserialize)]
struct DeviceResp {
    name: String,
    display_name: String,
    image_mime: String,
    image_url: String,
}

fn fetch_board_info(target: Arc<Mutex<Option<(String, String, String, String)>>>){
    execute(async move {
        use wasm_bindgen::JsCast;
        use wasm_bindgen_futures::JsFuture;
        use web_sys::{Request, RequestInit, Window, Response};

        let window: Window = web_sys::window().expect("no window");
        let opts = RequestInit::new();
        opts.set_method("GET");
        let request = Request::new_with_str_and_init("/device", &opts).unwrap();
        request.headers().set("Accept", "application/json").ok();

        let resp_val = JsFuture::from(window.fetch_with_request(&request)).await;
        if let Ok(val) = resp_val {
            let resp: Response = val.dyn_into().unwrap();
            if let Ok(text_js) = JsFuture::from(resp.text().unwrap()).await {
                let s = text_js.as_string().unwrap_or_default();
                if let Ok(parsed) = serde_json::from_str::<DeviceResp>(&s) {
                    *target.lock().unwrap() = Some((parsed.name, parsed.display_name, parsed.image_mime, parsed.image_url));
                }
            }
        }
    });
}

/// Fetch a text endpoint and return its body.
async fn http_get_text(path: &str) -> Result<String, JsValue> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    let window = web_sys::window().ok_or(JsValue::from_str("no window"))?;
    let resp_val = JsFuture::from(window.fetch_with_str(path)).await?;
    let resp: web_sys::Response = resp_val.dyn_into()?;
    let text = JsFuture::from(resp.text()?).await?;
    text.as_string().ok_or(JsValue::from_str("no text"))
}

fn load_mesh_from_bytes(bytes: &[u8]) -> Option<Mesh<()>> {
    if let Ok(m) = Mesh::<()>::from_stl(bytes, None) {
        return Some(m);
    }

    if let Ok(m) = Mesh::<()>::from_dxf(bytes, None) {
        return Some(m);
    }

    None
}

#[wasm_bindgen(start)]
pub async fn start() -> Result<(), JsValue> {
    console_log::init_with_level(Level::Debug).expect("failed to init logger");

    let web_options = eframe::WebOptions::default();

    // Bind eframe to the canvas declared in index.html.
    let document = web_sys::window()
        .expect("no window")
        .document()
        .expect("no document");
    let canvas = document
        .get_element_by_id("alumina_canvas")
        .expect("canvas not found")
        .dyn_into::<HtmlCanvasElement>()?;

    eframe::WebRunner::new()
        .start(
            canvas,
            web_options,
            Box::new(|cc| Ok(Box::new(AluminaApp::new(cc)))),
        )
        .await?;

    Ok(())
}

fn execute<F: Future<Output = ()> + 'static>(f: F) {
    wasm_bindgen_futures::spawn_local(f);
}
