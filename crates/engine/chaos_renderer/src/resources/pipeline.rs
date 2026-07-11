use crate::resources::shader::ShaderRef;
use crate::resources::texture::TextureFormat;
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

/// L'assemblage des sommets en primitives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveTopology {
    /// Des triangles indépendants, trois sommets chacun — le défaut.
    TriangleList,
    /// Une bande de triangles, chaque sommet en prolonge la précédente.
    TriangleStrip,
    /// Des segments indépendants, deux sommets chacun.
    LineList,
    /// Des points isolés, un sommet chacun.
    PointList,
}

/// L'élimination des faces selon leur orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CullMode {
    /// Aucune élimination — les deux faces sont rendues (double-sided).
    None,
    /// Les faces AVANT sont éliminées.
    Front,
    /// Les faces ARRIÈRE sont éliminées — le réglage classique des
    /// meshes fermés.
    Back,
}

/// La convention d'enroulement qui définit la face AVANT d'un triangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrontFace {
    /// Sens antihoraire (counter-clockwise) — la convention du moteur.
    Ccw,
    /// Sens horaire (clockwise).
    Cw,
}

/// Le test de profondeur du pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DepthCompare {
    /// Strictement plus proche — le défaut de tout rendu de scène.
    #[default]
    Less,
    /// Plus proche ou égal — les fonds plein écran (ciel) dessinés
    /// exactement à la profondeur maximale, là où rien n'a écrit.
    LessEqual,
    /// Toujours accepté — le test est IGNORÉ : les overlays (le debug
    /// rendering par-dessus tout) ; couplé à une profondeur en lecture
    /// seule, jamais à une écriture aveugle.
    Always,
}

/// Description d'un pipeline graphique, dans le vocabulaire du moteur.
/// La cible couleur est implicitement le format de la surface ; les cibles
/// offscreen viendront avec les phases de rendu avancées.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineDescriptor {
    /// Le label de diagnostic (logs, erreurs, outils de capture GPU).
    pub label: String,
    /// Le shader du pipeline — un nom de la `ShaderLibrary` ou une
    /// source WGSL inline.
    pub shader: ShaderRef,
    /// Le point d'entrée vertex du shader (`vs_main` par défaut).
    pub vertex_entry: String,
    /// Le point d'entrée fragment du shader (`fs_main` par défaut).
    pub fragment_entry: String,
    /// Le layout du vertex buffer ; `None` pour un pipeline sans
    /// géométrie (sommets générés par le shader).
    pub vertex_layout: Option<VertexLayout>,
    /// L'assemblage des primitives.
    pub topology: PrimitiveTopology,
    /// L'élimination des faces.
    pub cull_mode: CullMode,
    /// La convention de face avant.
    pub front_face: FrontFace,
    /// Le pipeline lit-il les ressources material (texture + sampler +
    /// uniforms du groupe 2) ? Voir `with_material`.
    pub material: bool,
    /// Le format de la cible couleur visée : `None` = le format de la
    /// SURFACE (le rendu fenêtre, défaut) ; `Some(format)` pour un
    /// pipeline destiné à une render target — les API graphiques exigent
    /// que le pipeline vise le format exact de sa cible.
    pub color_target: Option<TextureFormat>,
    /// Le pipeline dessine-t-il en TRANSPARENCE ? `true` = alpha
    /// blending et profondeur en lecture seule (le couplage V1 : une
    /// surface translucide ne doit pas occulter) ; `false` (défaut) =
    /// blend REPLACE, profondeur écrite.
    pub transparent: bool,
    /// Le test de profondeur (`Less` par défaut).
    pub depth_compare: DepthCompare,
    /// Le pipeline n'écrit QUE la profondeur (les passes d'ombre) :
    /// aucune cible couleur, aucun étage fragment — le vertex shader
    /// suffit. Incompatible avec `color_target` et `transparent`
    /// (jamais produit par le renderer).
    pub depth_only: bool,
    /// Le SECOND slot de vertex buffer, à cadence Instance — le layout
    /// des données par instance (les permutations instanciées). `None`
    /// (défaut) = un seul slot, le chemin classique.
    pub instance_layout: Option<VertexLayout>,
}

impl PipelineDescriptor {
    /// Descripteur aux défauts du moteur : entrées `vs_main`/`fs_main`,
    /// triangles, sans culling, enroulement CCW, sans groupe material.
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
            material: false,
            color_target: None,
            transparent: false,
            depth_compare: DepthCompare::Less,
            depth_only: false,
            instance_layout: None,
        }
    }

    /// Attache le layout du SECOND slot de vertex buffer, à cadence
    /// Instance — les données par instance des permutations instanciées.
    pub fn with_instance_layout(mut self, instance_layout: VertexLayout) -> Self {
        self.instance_layout = Some(instance_layout);
        self
    }

    /// Passe le pipeline en profondeur seule (les passes d'ombre) :
    /// aucune cible couleur, aucun étage fragment.
    pub fn with_depth_only(mut self) -> Self {
        self.depth_only = true;
        self
    }

    /// Attache le layout du vertex buffer consommé par le pipeline.
    pub fn with_vertex_layout(mut self, vertex_layout: VertexLayout) -> Self {
        self.vertex_layout = Some(vertex_layout);
        self
    }

    /// Règle l'élimination des faces.
    pub fn with_cull_mode(mut self, cull_mode: CullMode) -> Self {
        self.cull_mode = cull_mode;
        self
    }

    /// Ajoute le groupe(2) material (texture + sampler + MaterialUniforms)
    /// au layout du pipeline — le réglage des pipelines dont le shader lit
    /// les ressources material (ex. `chaos.textured`).
    pub fn with_material(mut self) -> Self {
        self.material = true;
        self
    }

    /// Vise une cible couleur hors écran : le pipeline devient utilisable
    /// vers les render targets de CE format (et plus vers la surface).
    pub fn with_color_target(mut self, format: TextureFormat) -> Self {
        self.color_target = Some(format);
        self
    }

    /// Passe le pipeline en transparence : alpha blending, profondeur en
    /// lecture seule.
    pub fn with_transparency(mut self) -> Self {
        self.transparent = true;
        self
    }

    /// Règle le test de profondeur.
    pub fn with_depth_compare(mut self, depth_compare: DepthCompare) -> Self {
        self.depth_compare = depth_compare;
        self
    }

    /// Règle le point d'entrée fragment (`fs_main` par défaut) — le
    /// levier des permutations masked (`fs_masked` élimine sous le
    /// cutoff) et des shaders multi-entrées en général.
    pub fn with_fragment_entry(mut self, fragment_entry: impl Into<String>) -> Self {
        self.fragment_entry = fragment_entry.into();
        self
    }

    /// Règle le point d'entrée vertex (`vs_main` par défaut) — le levier
    /// des permutations instanciées (`vs_instanced` lit les transforms
    /// depuis les attributs d'instance).
    pub fn with_vertex_entry(mut self, vertex_entry: impl Into<String>) -> Self {
        self.vertex_entry = vertex_entry.into();
        self
    }
}
