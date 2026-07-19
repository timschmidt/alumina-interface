use crate::fonts;
use csgrs::{mesh::Mesh, mesh::plane::Plane, sketch::Sketch, traits::CSG};
use egui::{self, DragValue};
use egui_node_graph2::*;
use nalgebra::{Point3, Vector3};

#[derive(Clone, Debug)]
pub struct EmptyUserResponse;

impl UserResponseTrait for EmptyUserResponse {}

/// Value type accepted by a graph port.
#[derive(PartialEq, Eq, Copy, Clone)]
pub enum DType {
    Mesh,
    Sketch,
    Scalar,
    Vec3,
    Text,
}

/// Run-time value carried by a port when the graph is evaluated.
#[derive(Clone, Debug)]
pub enum DValue {
    Mesh(Mesh<()>),
    Sketch(Sketch<()>),
    Scalar(f64),
    Vec3(Vector3<f64>),
    Text(String),
}

impl Default for DValue {
    fn default() -> Self {
        Self::Scalar(0.0)
    }
}

/// Node kind shown in the add-node menu.
#[derive(Copy, Clone, Debug)]
pub enum Template {
    /* ---- Sketch primitives ---- */
    Square,
    Rectangle,
    Circle,
    RoundedRectangle,
    Ellipse,
    RegularNgon,
    RightTriangle,
    Trapezoid,
    Star,
    TeardropSketch,
    EggSketch,
    Squircle,
    Keyhole,
    Reuleaux,
    Ring,
    PieSlice,
    Supershape,
    CircleWithKeyway,
    CircleWithFlat,
    CircleWithTwoFlats,
    Heart,
    Crescent,
    AirfoilNaca4,

    /* ---- Mesh primitives ---- */
    Cube,
    Cuboid,
    Sphere,
    Cylinder,
    Frustum,
    Octahedron,
    Icosahedron,
    Torus,
    EggMesh,
    TeardropMesh,
    TeardropCylinder,
    Ellipsoid,
    Arrow,

    /* ---- Booleans ---- */
    MeshUnion,
    MeshSubtract,
    MeshIntersect,
    SketchUnion,
    SketchSubtract,
    SketchIntersect,

    /* ---- Transforms (mesh) ---- */
    TranslateMesh,
    RotateMesh,
    ScaleMesh,
    MirrorMesh,
    CenterMesh,
    FloatMesh,
    InverseMesh,
    DistributeArcMesh,
    DistributeLinearMesh,
    DistributeGridMesh,

    /* ---- Transforms (sketch) ---- */
    TranslateSketch,
    RotateSketch,
    ScaleSketch,
    MirrorSketch,
    CenterSketch,
    FloatSketch,
    InverseSketch,
    DistributeArcSketch,
    DistributeLinearSketch,
    DistributeGridSketch,

    /* ---- 2D -> 3D ---- */
    Extrude,
    ExtrudeVector,
    Revolve,
    Loft,
    Sweep,

    /* ---- Mesh <-> Sketch helpers ---- */
    Flatten,
    Slice,

    /* ---- Field / lattice ops ---- */
    Gyroid,
    SchwarzP,
    SchwarzD,
    /* ---- Text ---- */
    TruetypeText,
}

impl Default for Template {
    fn default() -> Self {
        Template::Cube
    }
}

/// Return output sockets that have no downstream consumer.
pub fn graph_roots(graph: &GraphT) -> Vec<OutputId> {
    use std::collections::HashSet;
    let mut used = HashSet::<OutputId>::new();
    for input_id in graph.inputs.keys() {
        // Inputs may accept multiple connections, so inspect every source.
        for src_id in graph.connections(input_id) {
            used.insert(src_id);
        }
    }
    graph
        .outputs
        .keys()
        .filter(|oid| !used.contains(oid))
        .collect()
}

#[derive(Default)]
pub struct UserState;

#[derive(Default, Debug)]
pub struct NodeData {
    pub template: Template,
}

/// Define the color and label used for each socket type.
impl DataTypeTrait<UserState> for DType {
    fn data_type_color(&self, _u: &mut UserState) -> egui::Color32 {
        match self {
            DType::Mesh => egui::Color32::from_rgb(110, 200, 255),
            DType::Sketch => egui::Color32::from_rgb(120, 180, 120),
            DType::Scalar => egui::Color32::from_rgb(38, 109, 211),
            DType::Vec3 => egui::Color32::from_rgb(238, 207, 109),
            DType::Text => egui::Color32::from_rgb(200, 160, 240),
        }
    }
    fn name(&self) -> std::borrow::Cow<'_, str> {
        match self {
            DType::Mesh => "mesh".into(),
            DType::Sketch => "sketch".into(),
            DType::Scalar => "scalar".into(),
            DType::Vec3 => "vec3".into(),
            DType::Text => "text".into(),
        }
    }
}

// Node-port builders.
fn scalar_in(g: &mut Graph<NodeData, DType, DValue>, id: NodeId, name: &str, def: f64) {
    g.add_input_param(
        id,
        name.into(),
        DType::Scalar,
        DValue::Scalar(def),
        InputParamKind::ConnectionOrConstant,
        true,
    );
}
fn vec3_in(g: &mut Graph<NodeData, DType, DValue>, id: NodeId, name: &str, def: Vector3<f64>) {
    g.add_input_param(
        id,
        name.into(),
        DType::Vec3,
        DValue::Vec3(def),
        InputParamKind::ConnectionOrConstant,
        true,
    );
}
fn mesh_in(g: &mut Graph<NodeData, DType, DValue>, id: NodeId, name: &str) {
    g.add_input_param(
        id,
        name.into(),
        DType::Mesh,
        DValue::default(),
        InputParamKind::ConnectionOnly,
        true,
    );
}
fn sketch_in(g: &mut Graph<NodeData, DType, DValue>, id: NodeId, name: &str) {
    g.add_input_param(
        id,
        name.into(),
        DType::Sketch,
        DValue::default(),
        InputParamKind::ConnectionOnly,
        true,
    );
}
fn text_in(g: &mut Graph<NodeData, DType, DValue>, id: NodeId, name: &str, def: &str) {
    g.add_input_param(
        id,
        name.into(),
        DType::Text,
        DValue::Text(def.to_owned()),
        InputParamKind::ConnectionOrConstant,
        true,
    );
}
fn mesh_out(g: &mut Graph<NodeData, DType, DValue>, id: NodeId, name: &str) {
    g.add_output_param(id, name.into(), DType::Mesh);
}
fn sketch_out(g: &mut Graph<NodeData, DType, DValue>, id: NodeId, name: &str) {
    g.add_output_param(id, name.into(), DType::Sketch);
}

/// Configure menu labels, categories, and ports for each node kind.
impl NodeTemplateTrait for Template {
    type NodeData = NodeData;
    type DataType = DType;
    type ValueType = DValue;
    type UserState = UserState;
    type CategoryType = &'static str;

    fn node_finder_label(&self, _: &mut UserState) -> std::borrow::Cow<'_, str> {
        use Template::*;
        match self {
            /* sketch */
            Square => "Square".into(),
            Rectangle => "Rectangle".into(),
            Circle => "Circle".into(),
            RoundedRectangle => "Rounded Rect".into(),
            Ellipse => "Ellipse".into(),
            RegularNgon => "Regular n-gon".into(),
            RightTriangle => "Right triangle".into(),
            Trapezoid => "Trapezoid".into(),
            Star => "Star".into(),
            TeardropSketch => "Teardrop (2D)".into(),
            EggSketch => "Egg (2D)".into(),
            Squircle => "Squircle".into(),
            Keyhole => "Keyhole".into(),
            Reuleaux => "Reuleaux".into(),
            Ring => "Ring".into(),
            PieSlice => "Pie slice".into(),
            Supershape => "Supershape".into(),
            CircleWithKeyway => "Circle keyway".into(),
            CircleWithFlat => "Circle flat".into(),
            CircleWithTwoFlats => "Circle two flats".into(),
            Heart => "Heart".into(),
            Crescent => "Crescent".into(),
            AirfoilNaca4 => "Airfoil NACA4".into(),

            /* mesh */
            Cube => "Cube".into(),
            Cuboid => "Cuboid".into(),
            Sphere => "Sphere".into(),
            Cylinder => "Cylinder".into(),
            Frustum => "Frustum".into(),
            Octahedron => "Octahedron".into(),
            Icosahedron => "Icosahedron".into(),
            Torus => "Torus".into(),
            EggMesh => "Egg (3D)".into(),
            TeardropMesh => "Teardrop (3D)".into(),
            TeardropCylinder => "Teardrop cyl".into(),
            Ellipsoid => "Ellipsoid".into(),
            Arrow => "Arrow".into(),

            /* booleans */
            MeshUnion => "Mesh Union".into(),
            MeshSubtract => "Mesh Subtract".into(),
            MeshIntersect => "Mesh Intersect".into(),
            SketchUnion => "Sketch Union".into(),
            SketchSubtract => "Sketch Subtract".into(),
            SketchIntersect => "Sketch Intersect".into(),

            /* transforms mesh */
            TranslateMesh => "Translate Mesh".into(),
            RotateMesh => "Rotate Mesh".into(),
            ScaleMesh => "Scale Mesh".into(),
            MirrorMesh => "Mirror Mesh".into(),
            CenterMesh => "Center Mesh".into(),
            FloatMesh => "Float Mesh".into(),
            InverseMesh => "Inverse Mesh".into(),
            DistributeArcMesh => "Distribute Arc (Mesh)".into(),
            DistributeLinearMesh => "Distribute Linear (Mesh)".into(),
            DistributeGridMesh => "Distribute Grid (Mesh)".into(),

            /* transforms sketch */
            TranslateSketch => "Translate Sketch".into(),
            RotateSketch => "Rotate Sketch".into(),
            ScaleSketch => "Scale Sketch".into(),
            MirrorSketch => "Mirror Sketch".into(),
            CenterSketch => "Center Sketch".into(),
            FloatSketch => "Float Sketch".into(),
            InverseSketch => "Inverse Sketch".into(),
            DistributeArcSketch => "Distribute Arc (Sketch)".into(),
            DistributeLinearSketch => "Distribute Linear (Sketch)".into(),
            DistributeGridSketch => "Distribute Grid (Sketch)".into(),

            /* 2D -> 3D */
            Extrude => "Extrude".into(),
            ExtrudeVector => "Extrude Vector".into(),
            Revolve => "Revolve".into(),
            Loft => "Loft".into(),
            Sweep => "Sweep".into(),

            /* mesh<->sketch */
            Flatten => "Flatten".into(),
            Slice => "Slice".into(),

            Gyroid => "Gyroid".into(),
            SchwarzP => "Schwarz P".into(),
            SchwarzD => "Schwarz D".into(),
            TruetypeText => "TrueType text".into(),
        }
    }
    fn node_finder_categories(&self, _: &mut UserState) -> Vec<Self::CategoryType> {
        use Template::*;
        match self {
            Square | Rectangle | Circle | RoundedRectangle | Ellipse | RegularNgon
            | RightTriangle | Trapezoid | Star | TeardropSketch | EggSketch | Squircle
            | Keyhole | Reuleaux | Ring | PieSlice | Supershape | CircleWithKeyway
            | CircleWithFlat | CircleWithTwoFlats | Heart | Crescent | AirfoilNaca4
            | TruetypeText => {
                vec!["2D / Sketch"]
            }
            Cube | Cuboid | Sphere | Cylinder | Frustum | Octahedron | Icosahedron | Torus
            | EggMesh | TeardropMesh | TeardropCylinder | Ellipsoid | Arrow => vec!["3D / Mesh"],

            MeshUnion | MeshSubtract | MeshIntersect | SketchUnion | SketchSubtract
            | SketchIntersect => vec!["Boolean"],

            TranslateMesh
            | RotateMesh
            | ScaleMesh
            | MirrorMesh
            | CenterMesh
            | FloatMesh
            | InverseMesh
            | DistributeArcMesh
            | DistributeLinearMesh
            | DistributeGridMesh
            | TranslateSketch
            | RotateSketch
            | ScaleSketch
            | MirrorSketch
            | CenterSketch
            | FloatSketch
            | InverseSketch
            | DistributeArcSketch
            | DistributeLinearSketch
            | DistributeGridSketch => vec!["Transform"],

            Extrude | ExtrudeVector | Revolve | Loft | Sweep => vec!["2D -> 3D"],
            Flatten | Slice => vec!["Mesh/Sketch"],
            Gyroid | SchwarzP | SchwarzD => vec!["Lattice"],
        }
    }
    fn node_graph_label(&self, u: &mut UserState) -> String {
        self.node_finder_label(u).into()
    }
    fn user_data(&self, _: &mut UserState) -> Self::NodeData {
        NodeData { template: *self }
    }

    fn build_node(&self, g: &mut Graph<NodeData, DType, DValue>, _: &mut UserState, id: NodeId) {
        use Template::*;
        match self {
            /* ---- sketch primitives ---- */
            Square => {
                scalar_in(g, id, "width", 1.0);
                sketch_out(g, id, "out");
            }
            Rectangle => {
                scalar_in(g, id, "width", 1.0);
                scalar_in(g, id, "height", 1.0);
                sketch_out(g, id, "out");
            }
            Circle => {
                scalar_in(g, id, "radius", 1.0);
                scalar_in(g, id, "segments", 32.0);
                sketch_out(g, id, "out");
            }
            RoundedRectangle => {
                scalar_in(g, id, "width", 2.0);
                scalar_in(g, id, "height", 1.0);
                scalar_in(g, id, "corner_r", 0.2);
                scalar_in(g, id, "corner_segments", 8.0);
                sketch_out(g, id, "out");
            }
            Ellipse => {
                scalar_in(g, id, "width", 2.0);
                scalar_in(g, id, "height", 1.0);
                scalar_in(g, id, "segments", 32.0);
                sketch_out(g, id, "out");
            }
            RegularNgon => {
                scalar_in(g, id, "sides", 6.0);
                scalar_in(g, id, "radius", 1.0);
                sketch_out(g, id, "out");
            }
            RightTriangle => {
                scalar_in(g, id, "width", 1.0);
                scalar_in(g, id, "height", 1.0);
                sketch_out(g, id, "out");
            }
            Trapezoid => {
                scalar_in(g, id, "top_width", 1.0);
                scalar_in(g, id, "bottom_width", 2.0);
                scalar_in(g, id, "height", 1.0);
                scalar_in(g, id, "top_offset", 0.0);
                sketch_out(g, id, "out");
            }
            Star => {
                scalar_in(g, id, "points", 5.0);
                scalar_in(g, id, "outer_r", 1.0);
                scalar_in(g, id, "inner_r", 0.5);
                sketch_out(g, id, "out");
            }
            TeardropSketch => {
                scalar_in(g, id, "width", 1.0);
                scalar_in(g, id, "height", 1.5);
                scalar_in(g, id, "segments", 32.0);
                sketch_out(g, id, "out");
            }
            EggSketch => {
                scalar_in(g, id, "width", 1.0);
                scalar_in(g, id, "length", 2.0);
                scalar_in(g, id, "segments", 32.0);
                sketch_out(g, id, "out");
            }
            Squircle => {
                scalar_in(g, id, "width", 1.0);
                scalar_in(g, id, "height", 1.0);
                scalar_in(g, id, "segments", 32.0);
                sketch_out(g, id, "out");
            }
            Keyhole => {
                scalar_in(g, id, "circle_r", 0.5);
                scalar_in(g, id, "handle_w", 0.5);
                scalar_in(g, id, "handle_h", 1.0);
                scalar_in(g, id, "segments", 32.0);
                sketch_out(g, id, "out");
            }
            Reuleaux => {
                scalar_in(g, id, "sides", 3.0);
                scalar_in(g, id, "radius", 1.0);
                scalar_in(g, id, "arc_segments", 16.0);
                sketch_out(g, id, "out");
            }
            Ring => {
                scalar_in(g, id, "id", 1.0);
                scalar_in(g, id, "thickness", 0.2);
                scalar_in(g, id, "segments", 32.0);
                sketch_out(g, id, "out");
            }
            PieSlice => {
                scalar_in(g, id, "radius", 1.0);
                scalar_in(g, id, "start_deg", 0.0);
                scalar_in(g, id, "end_deg", 90.0);
                scalar_in(g, id, "segments", 32.0);
                sketch_out(g, id, "out");
            }
            Supershape => {
                scalar_in(g, id, "a", 1.0);
                scalar_in(g, id, "b", 1.0);
                scalar_in(g, id, "m", 5.0);
                scalar_in(g, id, "n1", 1.0);
                scalar_in(g, id, "n2", 1.0);
                scalar_in(g, id, "n3", 1.0);
                scalar_in(g, id, "segments", 128.0);
                sketch_out(g, id, "out");
            }
            CircleWithKeyway => {
                scalar_in(g, id, "radius", 1.0);
                scalar_in(g, id, "segments", 64.0);
                scalar_in(g, id, "key_w", 0.2);
                scalar_in(g, id, "key_d", 0.2);
                sketch_out(g, id, "out");
            }
            CircleWithFlat => {
                scalar_in(g, id, "radius", 1.0);
                scalar_in(g, id, "segments", 64.0);
                scalar_in(g, id, "flat_dist", 0.5);
                sketch_out(g, id, "out");
            }
            CircleWithTwoFlats => {
                scalar_in(g, id, "radius", 1.0);
                scalar_in(g, id, "segments", 64.0);
                scalar_in(g, id, "flat_dist", 0.5);
                sketch_out(g, id, "out");
            }
            Heart => {
                scalar_in(g, id, "width", 1.0);
                scalar_in(g, id, "height", 1.0);
                scalar_in(g, id, "segments", 64.0);
                sketch_out(g, id, "out");
            }
            Crescent => {
                scalar_in(g, id, "outer_r", 1.0);
                scalar_in(g, id, "inner_r", 0.5);
                scalar_in(g, id, "offset", 0.3);
                scalar_in(g, id, "segments", 32.0);
                sketch_out(g, id, "out");
            }
            AirfoilNaca4 => {
                scalar_in(g, id, "max_camber", 2.0);
                scalar_in(g, id, "camber_pos", 4.0);
                scalar_in(g, id, "thickness", 12.0);
                scalar_in(g, id, "chord", 1.0);
                scalar_in(g, id, "samples", 64.0);
                sketch_out(g, id, "out");
            }
            TruetypeText => {
                // String inputs for content and font selection
                text_in(g, id, "text", "Hello, world!");
                text_in(g, id, "font_family", ""); // search-as-you-type, or pick from dropdown
                text_in(g, id, "variant", "regular"); // default variant
                scalar_in(g, id, "height_mm", 20.0); // text cap height (mm)
                sketch_out(g, id, "out");
            }

            /* ---- mesh primitives ---- */
            Cube => {
                scalar_in(g, id, "size", 1.0);
                mesh_out(g, id, "out");
            }
            Cuboid => {
                scalar_in(g, id, "width", 1.0);
                scalar_in(g, id, "length", 2.0);
                scalar_in(g, id, "height", 1.0);
                mesh_out(g, id, "out");
            }
            Sphere => {
                scalar_in(g, id, "radius", 1.0);
                scalar_in(g, id, "segments", 24.0);
                scalar_in(g, id, "stacks", 12.0);
                mesh_out(g, id, "out");
            }
            Cylinder => {
                scalar_in(g, id, "radius", 1.0);
                scalar_in(g, id, "height", 1.0);
                scalar_in(g, id, "segments", 24.0);
                mesh_out(g, id, "out");
            }
            Frustum => {
                scalar_in(g, id, "r1", 1.0);
                scalar_in(g, id, "r2", 0.5);
                scalar_in(g, id, "height", 2.0);
                scalar_in(g, id, "segments", 24.0);
                mesh_out(g, id, "out");
            }
            Octahedron => {
                scalar_in(g, id, "radius", 1.0);
                mesh_out(g, id, "out");
            }
            Icosahedron => {
                scalar_in(g, id, "radius", 1.0);
                mesh_out(g, id, "out");
            }
            Torus => {
                scalar_in(g, id, "major_r", 2.0);
                scalar_in(g, id, "minor_r", 0.5);
                scalar_in(g, id, "segments_major", 32.0);
                scalar_in(g, id, "segments_minor", 16.0);
                mesh_out(g, id, "out");
            }
            EggMesh => {
                scalar_in(g, id, "width", 1.0);
                scalar_in(g, id, "length", 2.0);
                scalar_in(g, id, "rev_segments", 16.0);
                scalar_in(g, id, "outline_segments", 32.0);
                mesh_out(g, id, "out");
            }
            TeardropMesh => {
                scalar_in(g, id, "width", 1.0);
                scalar_in(g, id, "height", 2.0);
                scalar_in(g, id, "rev_segments", 16.0);
                scalar_in(g, id, "shape_segments", 32.0);
                mesh_out(g, id, "out");
            }
            TeardropCylinder => {
                scalar_in(g, id, "width", 1.0);
                scalar_in(g, id, "length", 2.0);
                scalar_in(g, id, "height", 1.0);
                scalar_in(g, id, "shape_segments", 32.0);
                mesh_out(g, id, "out");
            }
            Ellipsoid => {
                scalar_in(g, id, "rx", 1.0);
                scalar_in(g, id, "ry", 1.0);
                scalar_in(g, id, "rz", 2.0);
                scalar_in(g, id, "segments", 24.0);
                scalar_in(g, id, "stacks", 12.0);
                mesh_out(g, id, "out");
            }
            Arrow => {
                vec3_in(g, id, "start", Vector3::zeros());
                vec3_in(g, id, "direction", Vector3::new(0.0, 0.0, 2.0));
                scalar_in(g, id, "segments", 16.0);
                scalar_in(g, id, "orientation", 1.0);
                mesh_out(g, id, "out");
            }

            /* ---- booleans ---- */
            MeshUnion | MeshSubtract | MeshIntersect => {
                mesh_in(g, id, "A");
                mesh_in(g, id, "B");
                mesh_out(g, id, "out");
            }
            SketchUnion | SketchSubtract | SketchIntersect => {
                sketch_in(g, id, "A");
                sketch_in(g, id, "B");
                sketch_out(g, id, "out");
            }

            /* ---- mesh transforms ---- */
            TranslateMesh => {
                mesh_in(g, id, "in");
                vec3_in(g, id, "offset", Vector3::zeros());
                mesh_out(g, id, "out");
            }
            RotateMesh => {
                mesh_in(g, id, "in");
                vec3_in(g, id, "axis", Vector3::z());
                scalar_in(g, id, "angle_rad", 0.0);
                mesh_out(g, id, "out");
            }
            ScaleMesh => {
                mesh_in(g, id, "in");
                vec3_in(g, id, "factors", Vector3::new(1.0, 1.0, 1.0));
                mesh_out(g, id, "out");
            }
            MirrorMesh => {
                mesh_in(g, id, "in");
                vec3_in(g, id, "plane_normal", Vector3::x());
                scalar_in(g, id, "plane_w", 0.0);
                mesh_out(g, id, "out");
            }
            CenterMesh | FloatMesh | InverseMesh => {
                mesh_in(g, id, "in");
                mesh_out(g, id, "out");
            }
            DistributeArcMesh => {
                mesh_in(g, id, "in");
                scalar_in(g, id, "count", 3.0);
                scalar_in(g, id, "radius", 5.0);
                scalar_in(g, id, "start_deg", 0.0);
                scalar_in(g, id, "end_deg", 180.0);
                mesh_out(g, id, "out");
            }
            DistributeLinearMesh => {
                mesh_in(g, id, "in");
                scalar_in(g, id, "count", 3.0);
                vec3_in(g, id, "dir", Vector3::x());
                scalar_in(g, id, "spacing", 2.0);
                mesh_out(g, id, "out");
            }
            DistributeGridMesh => {
                mesh_in(g, id, "in");
                scalar_in(g, id, "rows", 2.0);
                scalar_in(g, id, "cols", 3.0);
                scalar_in(g, id, "dx", 2.0);
                scalar_in(g, id, "dy", 2.0);
                mesh_out(g, id, "out");
            }

            /* ---- sketch transforms ---- */
            TranslateSketch => {
                sketch_in(g, id, "in");
                vec3_in(g, id, "offset", Vector3::zeros());
                sketch_out(g, id, "out");
            }
            RotateSketch => {
                sketch_in(g, id, "in");
                vec3_in(g, id, "axis", Vector3::z());
                scalar_in(g, id, "angle_rad", 0.0);
                sketch_out(g, id, "out");
            }
            ScaleSketch => {
                sketch_in(g, id, "in");
                vec3_in(g, id, "factors", Vector3::new(1.0, 1.0, 1.0));
                sketch_out(g, id, "out");
            }
            MirrorSketch => {
                sketch_in(g, id, "in");
                vec3_in(g, id, "plane_normal", Vector3::x());
                scalar_in(g, id, "plane_w", 0.0);
                sketch_out(g, id, "out");
            }
            CenterSketch | FloatSketch | InverseSketch => {
                sketch_in(g, id, "in");
                sketch_out(g, id, "out");
            }
            DistributeArcSketch => {
                sketch_in(g, id, "in");
                scalar_in(g, id, "count", 3.0);
                scalar_in(g, id, "radius", 5.0);
                scalar_in(g, id, "start_deg", 0.0);
                scalar_in(g, id, "end_deg", 180.0);
                sketch_out(g, id, "out");
            }
            DistributeLinearSketch => {
                sketch_in(g, id, "in");
                scalar_in(g, id, "count", 3.0);
                vec3_in(g, id, "dir", Vector3::x());
                scalar_in(g, id, "spacing", 2.0);
                sketch_out(g, id, "out");
            }
            DistributeGridSketch => {
                sketch_in(g, id, "in");
                scalar_in(g, id, "rows", 2.0);
                scalar_in(g, id, "cols", 3.0);
                scalar_in(g, id, "dx", 2.0);
                scalar_in(g, id, "dy", 2.0);
                sketch_out(g, id, "out");
            }

            /* ---- 2D -> 3D ---- */
            Extrude => {
                sketch_in(g, id, "profile");
                scalar_in(g, id, "height", 1.0);
                mesh_out(g, id, "out");
            }
            ExtrudeVector => {
                sketch_in(g, id, "profile");
                vec3_in(g, id, "direction", Vector3::new(0.0, 0.0, 1.0));
                mesh_out(g, id, "out");
            }
            Revolve => {
                sketch_in(g, id, "profile");
                scalar_in(g, id, "angle_deg", 360.0);
                scalar_in(g, id, "segments", 16.0);
                mesh_out(g, id, "out");
            }
            Loft => {
                sketch_in(g, id, "bottom");
                sketch_in(g, id, "top");
                scalar_in(g, id, "caps", 1.0);
                mesh_out(g, id, "out");
            }
            Sweep => {
                sketch_in(g, id, "profile");
                vec3_in(g, id, "p0", Vector3::new(0.0, 0.0, 0.0));
                vec3_in(g, id, "p1", Vector3::new(0.0, 0.0, 5.0));
                mesh_out(g, id, "out");
            }

            /* mesh<->sketch */
            Flatten => {
                mesh_in(g, id, "in");
                sketch_out(g, id, "out");
            }
            Slice => {
                mesh_in(g, id, "in");
                vec3_in(g, id, "plane_normal", Vector3::z());
                scalar_in(g, id, "plane_w", 0.0);
                sketch_out(g, id, "out");
            }

            Gyroid | SchwarzP | SchwarzD => {
                mesh_in(g, id, "in");
                scalar_in(g, id, "resolution", 32.0);
                scalar_in(g, id, "period", 10.0);
                scalar_in(g, id, "iso_value", 0.0);
                mesh_out(g, id, "out");
            }
        }
    }
}

/// Enumerate templates available in the add-node menu.
pub struct AllTemplates;
impl NodeTemplateIter for AllTemplates {
    type Item = Template;
    fn all_kinds(&self) -> Vec<Self::Item> {
        use Template::*;
        vec![
            /* sketch */
            Square,
            Rectangle,
            Circle,
            RoundedRectangle,
            Ellipse,
            RegularNgon,
            RightTriangle,
            Trapezoid,
            Star,
            TeardropSketch,
            EggSketch,
            Squircle,
            Keyhole,
            Reuleaux,
            Ring,
            PieSlice,
            Supershape,
            CircleWithKeyway,
            CircleWithFlat,
            CircleWithTwoFlats,
            Heart,
            Crescent,
            AirfoilNaca4,
            TruetypeText,
            /* mesh */
            Cube,
            Cuboid,
            Sphere,
            Cylinder,
            Frustum,
            Octahedron,
            Icosahedron,
            Torus,
            EggMesh,
            TeardropMesh,
            TeardropCylinder,
            Ellipsoid,
            Arrow,
            /* booleans */
            MeshUnion,
            MeshSubtract,
            MeshIntersect,
            SketchUnion,
            SketchSubtract,
            SketchIntersect,
            /* transforms */
            TranslateMesh,
            RotateMesh,
            ScaleMesh,
            MirrorMesh,
            CenterMesh,
            FloatMesh,
            InverseMesh,
            DistributeArcMesh,
            DistributeLinearMesh,
            DistributeGridMesh,
            TranslateSketch,
            RotateSketch,
            ScaleSketch,
            MirrorSketch,
            CenterSketch,
            FloatSketch,
            InverseSketch,
            DistributeArcSketch,
            DistributeLinearSketch,
            DistributeGridSketch,
            /* 2D -> 3D */
            Extrude,
            ExtrudeVector,
            Revolve,
            Loft,
            Sweep,
            Flatten,
            Slice,
            Gyroid,
            SchwarzP,
            SchwarzD,
        ]
    }
}

/// Draw constant-value editors for graph inputs.
impl WidgetValueTrait for DValue {
    type NodeData = NodeData;
    type Response = EmptyUserResponse;
    type UserState = UserState;

    fn value_widget(
        &mut self,
        name: &str,
        _node_id: NodeId,
        ui: &mut egui::Ui,
        _user_state: &mut UserState,
        node_data: &NodeData,
    ) -> Vec<Self::Response> {
        match self {
            DValue::Scalar(x) => {
                ui.horizontal(|ui| {
                    ui.label(name);
                    ui.add(DragValue::new(x));
                });
            }
            DValue::Vec3(v) => {
                ui.label(name);
                ui.horizontal(|ui| {
                    ui.label("x");
                    ui.add(DragValue::new(&mut v.x));
                    ui.label("y");
                    ui.add(DragValue::new(&mut v.y));
                    ui.label("z");
                    ui.add(DragValue::new(&mut v.z));
                });
            }
            DValue::Sketch(_) => {
                ui.label("sketch");
            }
            DValue::Mesh(_) => {
                ui.label("mesh");
            }
            DValue::Text(s) => {
                // All text ports retain a direct editor.
                let is_truetype = matches!(node_data.template, Template::TruetypeText);
                let is_family = is_truetype && name == "font_family";
                let is_variant = is_truetype && name == "variant";

                ui.horizontal(|ui| {
                    ui.label(name);
                    ui.text_edit_singleline(s);
                });

                // Font-family ports also offer persisted browser fonts.
                if is_family {
                    #[cfg(target_arch = "wasm32")]
                    {
                        if let Ok(list) = fonts::list_persisted_ttf() {
                            // Present each family once, filtered by the typed query.
                            let mut families: Vec<String> =
                                list.into_iter().map(|p| p.family).collect();
                            families.sort();
                            families.dedup();

                            let query = s.to_ascii_lowercase();
                            let filtered = families
                                .into_iter()
                                .filter(|f| f.to_ascii_lowercase().contains(&query))
                                .collect::<Vec<_>>();

                            egui::ComboBox::from_id_salt("ttf_family_combo")
                                .selected_text(if s.is_empty() {
                                    "(pick font)"
                                } else {
                                    s.as_str()
                                })
                                .show_ui(ui, |ui| {
                                    for fam in &filtered {
                                        if ui.selectable_label(s == fam, fam).clicked() {
                                            *s = fam.clone();
                                        }
                                    }
                                });
                        } else {
                            ui.small("No persisted fonts found (localStorage).");
                        }
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        ui.small("Font picker available on web build (wasm32).");
                    }
                }

                // Variant ports offer the common Google Fonts variants.
                if is_variant {
                    let current = if s.is_empty() { "regular" } else { s.as_str() };
                    egui::ComboBox::from_id_salt("ttf_variant_combo")
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for c in ["regular", "italic", "700", "700italic"] {
                                if ui.selectable_label(s == c, c).clicked() {
                                    s.clear();
                                    s.push_str(c);
                                }
                            }
                        });
                }
            }
        }
        Vec::new()
    }
}

/// Nodes do not add a bottom-panel UI.
impl NodeDataTrait for NodeData {
    type Response = EmptyUserResponse;
    type UserState = UserState;
    type DataType = DType;
    type ValueType = DValue;

    fn bottom_ui(
        &self,
        _ui: &mut egui::Ui,
        _id: NodeId,
        _graph: &Graph<NodeData, DType, DValue>,
        _state: &mut UserState,
    ) -> Vec<NodeResponse<EmptyUserResponse, Self>> {
        Vec::new()
    }
}

// Graph evaluation.

type Cache = std::collections::HashMap<OutputId, DValue>;
type GraphT = Graph<NodeData, DType, DValue>;

/// Evaluate one graph output as the 3D mesh displayed by the viewer.
pub fn evaluate(graph: &GraphT, root: OutputId) -> anyhow::Result<Mesh<()>> {
    let mut cache = Cache::new();
    let val = eval_rec(graph, root, &mut cache)?;
    match val {
        DValue::Mesh(mesh) => Ok(mesh),
        _ => anyhow::bail!("root output does not evaluate to a mesh"),
    }
}

fn as_usize(x: f64) -> usize {
    if x <= 0.0 { 0 } else { x.round() as usize }
}
fn as_bool(x: f64) -> bool {
    x.abs() > std::f64::EPSILON
}

fn eval_rec(graph: &GraphT, out: OutputId, cache: &mut Cache) -> anyhow::Result<DValue> {
    if let Some(v) = cache.get(&out) {
        return Ok(v.clone());
    }
    let node_id = graph[out].node;
    log::warn!("node_id: {:#?}", node_id);
    let node = &graph[node_id];
    log::warn!("node: {:#?}", node);

    // Resolve a connected input recursively or use its constant value.
    let mut get = |name: &str| -> anyhow::Result<DValue> {
        let in_id = node.get_input(name)?;
        if let Some(src) = graph.connections(in_id).first() {
            eval_rec(graph, *src, cache)
        } else {
            Ok(graph[in_id].value.clone())
        }
    };

    use Template::*;
    let value = match node.user_data.template {
        /* ---- sketch primitives ---- */
        Square => {
            let w = get("width")?.scalar()?;
            DValue::Sketch(Sketch::square(w.into(), None))
        }
        Rectangle => {
            let w = get("width")?.scalar()?;
            let h = get("height")?.scalar()?;
            DValue::Sketch(Sketch::rectangle(w.into(), h.into(), None))
        }
        Circle => {
            let r = get("radius")?.scalar()?;
            let segs = as_usize(get("segments")?.scalar()?);
            DValue::Sketch(Sketch::circle(r.into(), segs, None))
        }
        RoundedRectangle => {
            let w = get("width")?.scalar()?;
            let h = get("height")?.scalar()?;
            let c = get("corner_r")?.scalar()?;
            let segs = as_usize(get("corner_segments")?.scalar()?);
            DValue::Sketch(Sketch::rounded_rectangle(
                w.into(),
                h.into(),
                c.into(),
                segs,
                None,
            ))
        }
        Ellipse => {
            let w = get("width")?.scalar()?;
            let h = get("height")?.scalar()?;
            let segs = as_usize(get("segments")?.scalar()?);
            DValue::Sketch(Sketch::ellipse(w.into(), h.into(), segs, None))
        }
        RegularNgon => {
            let sides = as_usize(get("sides")?.scalar()?);
            let r = get("radius")?.scalar()?;
            DValue::Sketch(Sketch::regular_ngon(sides, r.into(), None))
        }
        RightTriangle => {
            let w = get("width")?.scalar()?;
            let h = get("height")?.scalar()?;
            DValue::Sketch(Sketch::right_triangle(w.into(), h.into(), None))
        }
        Trapezoid => {
            let t = get("top_width")?.scalar()?;
            let b = get("bottom_width")?.scalar()?;
            let h = get("height")?.scalar()?;
            let off = get("top_offset")?.scalar()?;
            DValue::Sketch(Sketch::trapezoid(
                t.into(),
                b.into(),
                h.into(),
                off.into(),
                None,
            ))
        }
        Star => {
            let n = as_usize(get("points")?.scalar()?);
            let outer = get("outer_r")?.scalar()?;
            let inner = get("inner_r")?.scalar()?;
            DValue::Sketch(Sketch::star(n, outer.into(), inner.into(), None))
        }
        TeardropSketch => {
            let w = get("width")?.scalar()?;
            let h = get("height")?.scalar()?;
            let segs = as_usize(get("segments")?.scalar()?);
            DValue::Sketch(Sketch::teardrop(w.into(), h.into(), segs, None))
        }
        EggSketch => {
            let w = get("width")?.scalar()?;
            let l = get("length")?.scalar()?;
            let segs = as_usize(get("segments")?.scalar()?);
            DValue::Sketch(Sketch::egg(w.into(), l.into(), segs, None))
        }
        Squircle => {
            let w = get("width")?.scalar()?;
            let h = get("height")?.scalar()?;
            let segs = as_usize(get("segments")?.scalar()?);
            DValue::Sketch(Sketch::squircle(w.into(), h.into(), segs, None))
        }
        Keyhole => {
            let cr = get("circle_r")?.scalar()?;
            let hw = get("handle_w")?.scalar()?;
            let hh = get("handle_h")?.scalar()?;
            let segs = as_usize(get("segments")?.scalar()?);
            DValue::Sketch(Sketch::keyhole(cr.into(), hw.into(), hh.into(), segs, None))
        }
        Reuleaux => {
            let s = as_usize(get("sides")?.scalar()?);
            let r = get("radius")?.scalar()?;
            let arcs = as_usize(get("arc_segments")?.scalar()?);
            DValue::Sketch(Sketch::reuleaux(s, r.into(), arcs, None))
        }
        Ring => {
            let id = get("id")?.scalar()?;
            let th = get("thickness")?.scalar()?;
            let segs = as_usize(get("segments")?.scalar()?);
            DValue::Sketch(Sketch::ring(id.into(), th.into(), segs, None))
        }
        PieSlice => {
            let r = get("radius")?.scalar()?;
            let s = get("start_deg")?.scalar()?;
            let e = get("end_deg")?.scalar()?;
            let segs = as_usize(get("segments")?.scalar()?);
            DValue::Sketch(Sketch::pie_slice(r.into(), s.into(), e.into(), segs, None))
        }
        Supershape => {
            let a = get("a")?.scalar()?;
            let b = get("b")?.scalar()?;
            let m = get("m")?.scalar()?;
            let n1 = get("n1")?.scalar()?;
            let n2 = get("n2")?.scalar()?;
            let n3 = get("n3")?.scalar()?;
            let segs = as_usize(get("segments")?.scalar()?);
            DValue::Sketch(Sketch::supershape(
                a.into(),
                b.into(),
                m.into(),
                n1.into(),
                n2.into(),
                n3.into(),
                segs,
                None,
            ))
        }
        CircleWithKeyway => {
            let r = get("radius")?.scalar()?;
            let segs = as_usize(get("segments")?.scalar()?);
            let k_w = get("key_w")?.scalar()?;
            let k_d = get("key_d")?.scalar()?;
            DValue::Sketch(Sketch::circle_with_keyway(
                r.into(),
                segs,
                k_w.into(),
                k_d.into(),
                None,
            ))
        }
        CircleWithFlat => {
            let r = get("radius")?.scalar()?;
            let segs = as_usize(get("segments")?.scalar()?);
            let dist = get("flat_dist")?.scalar()?;
            DValue::Sketch(Sketch::circle_with_flat(r.into(), segs, dist.into(), None))
        }
        CircleWithTwoFlats => {
            let r = get("radius")?.scalar()?;
            let segs = as_usize(get("segments")?.scalar()?);
            let dist = get("flat_dist")?.scalar()?;
            DValue::Sketch(Sketch::circle_with_two_flats(
                r.into(),
                segs,
                dist.into(),
                None,
            ))
        }
        Heart => {
            let w = get("width")?.scalar()?;
            let h = get("height")?.scalar()?;
            let segs = as_usize(get("segments")?.scalar()?);
            DValue::Sketch(Sketch::heart(w.into(), h.into(), segs, None))
        }
        Crescent => {
            let o = get("outer_r")?.scalar()?;
            let i = get("inner_r")?.scalar()?;
            let off = get("offset")?.scalar()?;
            let segs = as_usize(get("segments")?.scalar()?);
            DValue::Sketch(Sketch::crescent(o.into(), i.into(), off.into(), segs, None))
        }
        AirfoilNaca4 => {
            let mc = get("max_camber")?.scalar()?;
            let cp = get("camber_pos")?.scalar()?;
            let th = get("thickness")?.scalar()?;
            let chord = get("chord")?.scalar()?;
            let samples = as_usize(get("samples")?.scalar()?);
            DValue::Sketch(Sketch::airfoil_naca4(
                mc.into(),
                cp.into(),
                th.into(),
                chord.into(),
                samples,
                None,
            ))
        }
        TruetypeText => {
            let content = get("text")?.text()?;
            let family = get("font_family")?.text()?.trim().to_owned();
            let variant = get("variant")?.text()?;
            let height = get("height_mm")?.scalar()?;

            // Resolve the selected font from browser-local storage.
            #[cfg(target_arch = "wasm32")]
            {
                let maybe_bytes = fonts::load_persisted_ttf(&family, &variant)
                    .map_err(|e| anyhow::anyhow!("load_persisted_ttf failed: {:?}", e))?;
                match maybe_bytes {
                    Some(bytes) => {
                        DValue::Sketch(Sketch::text(&content, &bytes, height.into(), None))
                    }
                    None => {
                        anyhow::bail!(
                            "No persisted TTF for \"{family}:{variant}\". Pick a font that’s saved to localStorage."
                        );
                    }
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                anyhow::bail!("TrueType text requires web build (wasm32) with persisted fonts.");
            }
        }

        /* ---- mesh primitives ---- */
        ,
        Cube => {
            let s = get("size")?.scalar()?;
            DValue::Mesh(Mesh::cube(s.into(), None))
        }
        Cuboid => {
            let w = get("width")?.scalar()?;
            let l = get("length")?.scalar()?;
            let h = get("height")?.scalar()?;
            DValue::Mesh(Mesh::cuboid(w.into(), l.into(), h.into(), None))
        }
        Sphere => {
            let r = get("radius")?.scalar()?;
            let seg = as_usize(get("segments")?.scalar()?);
            let st = as_usize(get("stacks")?.scalar()?);
            DValue::Mesh(Mesh::sphere(r.into(), seg, st, None))
        }
        Cylinder => {
            let r = get("radius")?.scalar()?;
            let h = get("height")?.scalar()?;
            let seg = as_usize(get("segments")?.scalar()?);
            DValue::Mesh(Mesh::cylinder(r.into(), h.into(), seg, None))
        }
        Frustum => {
            let r1 = get("r1")?.scalar()?;
            let r2 = get("r2")?.scalar()?;
            let h = get("height")?.scalar()?;
            let seg = as_usize(get("segments")?.scalar()?);
            DValue::Mesh(Mesh::frustum(r1.into(), r2.into(), h.into(), seg, None))
        }
        Octahedron => {
            let r = get("radius")?.scalar()?;
            DValue::Mesh(Mesh::octahedron(r.into(), None))
        }
        Icosahedron => {
            let r = get("radius")?.scalar()?;
            DValue::Mesh(Mesh::icosahedron(r.into(), None))
        }
        Torus => {
            let mr = get("major_r")?.scalar()?;
            let nr = get("minor_r")?.scalar()?;
            let sm = as_usize(get("segments_major")?.scalar()?);
            let sn = as_usize(get("segments_minor")?.scalar()?);
            DValue::Mesh(Mesh::torus(mr.into(), nr.into(), sm, sn, None))
        }
        EggMesh => {
            let w = get("width")?.scalar()?;
            let l = get("length")?.scalar()?;
            let rs = as_usize(get("rev_segments")?.scalar()?);
            let os = as_usize(get("outline_segments")?.scalar()?);
            DValue::Mesh(Mesh::egg(w.into(), l.into(), rs, os, None))
        }
        TeardropMesh => {
            let w = get("width")?.scalar()?;
            let h = get("height")?.scalar()?;
            let rs = as_usize(get("rev_segments")?.scalar()?);
            let ss = as_usize(get("shape_segments")?.scalar()?);
            DValue::Mesh(Mesh::teardrop(w.into(), h.into(), rs, ss, None))
        }
        TeardropCylinder => {
            let w = get("width")?.scalar()?;
            let l = get("length")?.scalar()?;
            let h = get("height")?.scalar()?;
            let ss = as_usize(get("shape_segments")?.scalar()?);
            DValue::Mesh(Mesh::teardrop_cylinder(
                w.into(),
                l.into(),
                h.into(),
                ss,
                None,
            ))
        }
        Ellipsoid => {
            let rx = get("rx")?.scalar()?;
            let ry = get("ry")?.scalar()?;
            let rz = get("rz")?.scalar()?;
            let seg = as_usize(get("segments")?.scalar()?);
            let st = as_usize(get("stacks")?.scalar()?);
            DValue::Mesh(Mesh::ellipsoid(
                rx.into(),
                ry.into(),
                rz.into(),
                seg,
                st,
                None,
            ))
        }
        Arrow => {
            let s = get("start")?.vec3()?;
            let d = get("direction")?.vec3()?;
            let seg = as_usize(get("segments")?.scalar()?);
            let orient = as_bool(get("orientation")?.scalar()?);
            DValue::Mesh(Mesh::arrow(
                Point3::new(s.x.into(), s.y.into(), s.z.into()),
                Vector3::new(d.x.into(), d.y.into(), d.z.into()),
                seg,
                orient,
                None,
            ))
        }

        /* ---- booleans ---- */
        ,
        MeshUnion => {
            let a = get("A")?.mesh()?;
            let b = get("B")?.mesh()?;
            DValue::Mesh(a.union(&b))
        }
        MeshSubtract => {
            let a = get("A")?.mesh()?;
            let b = get("B")?.mesh()?;
            DValue::Mesh(a.difference(&b))
        }
        MeshIntersect => {
            let a = get("A")?.mesh()?;
            let b = get("B")?.mesh()?;
            DValue::Mesh(a.intersection(&b))
        }
        SketchUnion => {
            let a = get("A")?.sketch()?;
            let b = get("B")?.sketch()?;
            DValue::Sketch(a.union(&b))
        }
        SketchSubtract => {
            let a = get("A")?.sketch()?;
            let b = get("B")?.sketch()?;
            DValue::Sketch(a.difference(&b))
        }
        SketchIntersect => {
            let a = get("A")?.sketch()?;
            let b = get("B")?.sketch()?;
            DValue::Sketch(a.intersection(&b))
        }

        /* ---- mesh transforms ---- */
        ,
        TranslateMesh => {
            let m = get("in")?.mesh()?;
            let o = get("offset")?.vec3()?;
            DValue::Mesh(m.translate(o.x.into(), o.y.into(), o.z.into()))
        }
        RotateMesh => {
            let m = get("in")?.mesh()?;
            let axis = get("axis")?.vec3()?.normalize();
            let ang = get("angle_rad")?.scalar()?;
            let deg = ang.to_degrees();
            DValue::Mesh(m.rotate(axis.x * deg, axis.y * deg, axis.z * deg))
        }
        ScaleMesh => {
            let m = get("in")?.mesh()?;
            let f = get("factors")?.vec3()?;
            DValue::Mesh(m.scale(f.x.into(), f.y.into(), f.z.into()))
        }
        MirrorMesh => {
            let m = get("in")?.mesh()?;
            let n = get("plane_normal")?.vec3()?;
            let w = get("plane_w")?.scalar()?;
            DValue::Mesh(m.mirror(Plane::from_normal(
                Vector3::new(n.x.into(), n.y.into(), n.z.into()),
                w.into(),
            )))
        }
        CenterMesh => {
            let m = get("in")?.mesh()?;
            DValue::Mesh(m.center())
        }
        FloatMesh => {
            let m = get("in")?.mesh()?;
            DValue::Mesh(m.float())
        }
        InverseMesh => {
            let m = get("in")?.mesh()?;
            DValue::Mesh(m.inverse())
        }
        DistributeArcMesh => {
            let m = get("in")?.mesh()?;
            let count = as_usize(get("count")?.scalar()?);
            let r = get("radius")?.scalar()?;
            let s = get("start_deg")?.scalar()?;
            let e = get("end_deg")?.scalar()?;
            DValue::Mesh(m.distribute_arc(count, r.into(), s.into(), e.into()))
        }
        DistributeLinearMesh => {
            let m = get("in")?.mesh()?;
            let count = as_usize(get("count")?.scalar()?);
            let dir = get("dir")?.vec3()?;
            let spacing = get("spacing")?.scalar()?;
            DValue::Mesh(m.distribute_linear(
                count,
                Vector3::new(dir.x.into(), dir.y.into(), dir.z.into()),
                spacing.into(),
            ))
        }
        DistributeGridMesh => {
            let m = get("in")?.mesh()?;
            let rows = as_usize(get("rows")?.scalar()?);
            let cols = as_usize(get("cols")?.scalar()?);
            let dx = get("dx")?.scalar()?;
            let dy = get("dy")?.scalar()?;
            DValue::Mesh(m.distribute_grid(rows, cols, dx.into(), dy.into()))
        }

        /* ---- sketch transforms ---- */
        ,
        TranslateSketch => {
            let s = get("in")?.sketch()?;
            let o = get("offset")?.vec3()?;
            DValue::Sketch(s.translate(o.x.into(), o.y.into(), o.z.into()))
        }
        RotateSketch => {
            let s = get("in")?.sketch()?;
            let axis = get("axis")?.vec3()?.normalize();
            let ang = get("angle_rad")?.scalar()?;
            let deg = ang.to_degrees();
            DValue::Sketch(s.rotate(axis.x * deg, axis.y * deg, axis.z * deg))
        }
        ScaleSketch => {
            let s = get("in")?.sketch()?;
            let f = get("factors")?.vec3()?;
            DValue::Sketch(s.scale(f.x.into(), f.y.into(), f.z.into()))
        }
        MirrorSketch => {
            let s = get("in")?.sketch()?;
            let n = get("plane_normal")?.vec3()?;
            let w = get("plane_w")?.scalar()?;
            DValue::Sketch(s.mirror(Plane::from_normal(
                Vector3::new(n.x.into(), n.y.into(), n.z.into()),
                w.into(),
            )))
        }
        CenterSketch => {
            let s = get("in")?.sketch()?;
            DValue::Sketch(s.center())
        }
        FloatSketch => {
            let s = get("in")?.sketch()?;
            DValue::Sketch(s.float())
        }
        InverseSketch => {
            let s = get("in")?.sketch()?;
            DValue::Sketch(s.inverse())
        }
        DistributeArcSketch => {
            let s = get("in")?.sketch()?;
            let count = as_usize(get("count")?.scalar()?);
            let r = get("radius")?.scalar()?;
            let st = get("start_deg")?.scalar()?;
            let en = get("end_deg")?.scalar()?;
            DValue::Sketch(s.distribute_arc(count, r.into(), st.into(), en.into()))
        }
        DistributeLinearSketch => {
            let s = get("in")?.sketch()?;
            let count = as_usize(get("count")?.scalar()?);
            let dir = get("dir")?.vec3()?;
            let spacing = get("spacing")?.scalar()?;
            DValue::Sketch(s.distribute_linear(
                count,
                Vector3::new(dir.x.into(), dir.y.into(), dir.z.into()),
                spacing.into(),
            ))
        }
        DistributeGridSketch => {
            let s = get("in")?.sketch()?;
            let rows = as_usize(get("rows")?.scalar()?);
            let cols = as_usize(get("cols")?.scalar()?);
            let dx = get("dx")?.scalar()?;
            let dy = get("dy")?.scalar()?;
            DValue::Sketch(s.distribute_grid(rows, cols, dx.into(), dy.into()))
        }

        /* ---- 2D -> 3D ---- */
        ,
        Extrude => {
            let s = get("profile")?.sketch()?;
            let h = get("height")?.scalar()?;
            DValue::Mesh(s.extrude(h.into()))
        }
        ExtrudeVector => {
            let s = get("profile")?.sketch()?;
            let d = get("direction")?.vec3()?;
            DValue::Mesh(s.extrude_vector(Vector3::new(d.x.into(), d.y.into(), d.z.into())))
        }
        Revolve => {
            let s = get("profile")?.sketch()?;
            let a = get("angle_deg")?.scalar()?;
            let seg = as_usize(get("segments")?.scalar()?);
            DValue::Mesh(s.revolve(a.into(), seg).unwrap())
        }
        Loft => {
            let btm = get("bottom")?.mesh()?;
            let top = get("top")?.mesh()?;
            let caps = as_bool(get("caps")?.scalar()?);
            // Loft the first polygon from each input sketch.
            DValue::Mesh(Sketch::loft(&btm.polygons[0], &top.polygons[0], caps).unwrap())
        }
        Sweep => {
            let s = get("profile")?.sketch()?;
            let p0 = get("p0")?.vec3()?;
            let p1 = get("p1")?.vec3()?;
            let path = [
                Point3::new(p0.x.into(), p0.y.into(), p0.z.into()),
                Point3::new(p1.x.into(), p1.y.into(), p1.z.into()),
            ];
            DValue::Mesh(s.sweep(&path))
        }

        /* mesh<->sketch */
        ,
        Flatten => {
            let m = get("in")?.mesh()?;
            DValue::Sketch(m.flatten())
        }
        Slice => {
            let m = get("in")?.mesh()?;
            let n = get("plane_normal")?.vec3()?;
            let w = get("plane_w")?.scalar()?;
            let plane =
                Plane::from_normal(Vector3::new(n.x.into(), n.y.into(), n.z.into()), w.into());
            DValue::Sketch(m.slice(plane))
        }

        Gyroid => {
            let m = get("in")?.mesh()?;
            let res = as_usize(get("resolution")?.scalar()?);
            let period = get("period")?.scalar()?;
            let iso = get("iso_value")?.scalar()?;
            DValue::Mesh(m.gyroid(res, period.into(), iso.into(), None))
        }
        SchwarzP => {
            let m = get("in")?.mesh()?;
            let res = as_usize(get("resolution")?.scalar()?);
            let period = get("period")?.scalar()?;
            let iso = get("iso_value")?.scalar()?;
            DValue::Mesh(m.schwarz_p(res, period.into(), iso.into(), None))
        }
        SchwarzD => {
            let m = get("in")?.mesh()?;
            let res = as_usize(get("resolution")?.scalar()?);
            let period = get("period")?.scalar()?;
            let iso = get("iso_value")?.scalar()?;
            DValue::Mesh(m.schwarz_d(res, period.into(), iso.into(), None))
        }
    };

    cache.insert(out, value.clone());
    log::warn!("value: {:#?}", value);
    log::warn!("cache: {:#?}", cache);
    Ok(value)
}

/// Extract typed values while evaluating connected ports.
trait AsTyped {
    fn scalar(self) -> anyhow::Result<f64>;
    fn vec3(self) -> anyhow::Result<Vector3<f64>>;
    fn mesh(self) -> anyhow::Result<Mesh<()>>;
    fn sketch(self) -> anyhow::Result<Sketch<()>>;
    fn text(self) -> anyhow::Result<String>;
}
impl AsTyped for DValue {
    fn scalar(self) -> anyhow::Result<f64> {
        if let DValue::Scalar(x) = self {
            Ok(x)
        } else {
            anyhow::bail!("expected scalar")
        }
    }
    fn vec3(self) -> anyhow::Result<Vector3<f64>> {
        if let DValue::Vec3(v) = self {
            Ok(v)
        } else {
            anyhow::bail!("expected vec3")
        }
    }
    fn mesh(self) -> anyhow::Result<Mesh<()>> {
        if let DValue::Mesh(m) = self {
            Ok(m)
        } else {
            anyhow::bail!("expected mesh")
        }
    }
    fn sketch(self) -> anyhow::Result<Sketch<()>> {
        if let DValue::Sketch(s) = self {
            Ok(s)
        } else {
            anyhow::bail!("expected sketch")
        }
    }
    fn text(self) -> anyhow::Result<String> {
        if let DValue::Text(s) = self {
            Ok(s)
        } else {
            anyhow::bail!("expected text")
        }
    }
}
