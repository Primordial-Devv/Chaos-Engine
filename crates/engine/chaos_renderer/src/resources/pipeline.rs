use crate::resources::shader::ShaderRef;
use crate::resources::vertex::VertexLayout;

/// Identifiant opaque d'un pipeline créé par le backend.
/// Index croissant ; la suppression et les générations viendront avec la
/// gestion de ressources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PipelineHandle(pub(crate) u32);

impl PipelineHandle {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveTopology {
    TriangleList,
    TriangleStrip,
    LineList,
    PointList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CullMode {
    None,
    Front,
    Back,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrontFace {
    Ccw,
    Cw,
}

/// Description d'un pipeline graphique, dans le vocabulaire du moteur.
/// La cible couleur est implicitement le format de la surface ; les cibles
/// offscreen viendront avec les phases de rendu avancées.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineDescriptor {
    pub label: String,
    pub shader: ShaderRef,
    pub vertex_entry: String,
    pub fragment_entry: String,
    pub vertex_layout: Option<VertexLayout>,
    pub topology: PrimitiveTopology,
    pub cull_mode: CullMode,
    pub front_face: FrontFace,
}

impl PipelineDescriptor {
    pub fn new(label: impl Into<String>, shader: impl Into<ShaderRef>) -> Self {
        Self {
            label: label.into(),
            shader: shader.into(),
            vertex_entry: String::from("vs_main"),
            fragment_entry: String::from("fs_main"),
            vertex_layout: None,
            topology: PrimitiveTopology::TriangleList,
            cull_mode: CullMode::None,
            front_face: FrontFace::Ccw,
        }
    }

    pub fn with_vertex_layout(mut self, vertex_layout: VertexLayout) -> Self {
        self.vertex_layout = Some(vertex_layout);
        self
    }
}
