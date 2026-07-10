use std::collections::HashMap;

use chaos_core::math::Mat4;
use chaos_core::{ChaosError, ChaosResult, Color};
use log::{debug, info, warn};

use crate::backend::{GraphicsBackend, create_backend};
use crate::config::RendererConfig;
use crate::frame::{DrawCommand, FrameDraw, FrameOutcome, FramePlan};
use crate::geometry::{Geometry, TexturedGeometry};
use crate::material::{MaterialDescriptor, MaterialHandle, MaterialRecord};
use crate::mesh::{MeshHandle, MeshRecord};
use crate::pool::{PoolHandle, ResourcePool};
use crate::queue::RenderQueue;
use crate::resources::{
    BufferDescriptor, BufferHandle, ColorVertex, MaterialBindingDescriptor, PipelineDescriptor,
    PipelineHandle, SamplerDescriptor, SamplerHandle, ShaderRef, TextureDescriptor, TextureFormat,
    TextureHandle, TexturedVertex, VertexLayout,
};
use crate::shaders::ShaderLibrary;
use crate::target::SurfaceTarget;

/// Renderer du moteur : orchestre un backend graphique interchangeable et
/// tient le registre des meshes (géométrie résidente GPU).
///
/// L'API ne parle que le vocabulaire de chaos_core ; le backend concret
/// (wgpu aujourd'hui) reste un détail d'implémentation interne.
pub struct Renderer {
    backend: Box<dyn GraphicsBackend>,
    shaders: ShaderLibrary,
    meshes: ResourcePool<MeshRecord>,
    materials: ResourcePool<MaterialRecord>,
    texture_cache: HashMap<String, TextureHandle>,
    fallback_sampler: Option<SamplerHandle>,
    clear_color: Color,
    view_projection: Mat4,
    surface_size: (u32, u32),
    queue: RenderQueue,
}

impl Renderer {
    /// Attache le renderer à une cible de présentation et initialise le GPU.
    pub fn attach(
        target: impl SurfaceTarget + 'static,
        config: RendererConfig,
    ) -> ChaosResult<Self> {
        let backend = create_backend(Box::new(target), config)?;
        info!("renderer ready: {}", backend.description());
        let mut renderer = Self::with_backend(backend);
        renderer.surface_size = (config.width, config.height);
        Ok(renderer)
    }

    pub(crate) fn with_backend(backend: Box<dyn GraphicsBackend>) -> Self {
        Self {
            backend,
            shaders: ShaderLibrary::with_builtins(),
            meshes: ResourcePool::new(),
            materials: ResourcePool::new(),
            texture_cache: HashMap::new(),
            fallback_sampler: None,
            clear_color: Color::BLACK,
            view_projection: Mat4::IDENTITY,
            surface_size: (1, 1),
            queue: RenderQueue::new(),
        }
    }

    pub fn description(&self) -> String {
        self.backend.description()
    }

    pub fn shaders(&self) -> &ShaderLibrary {
        &self.shaders
    }

    pub fn shaders_mut(&mut self) -> &mut ShaderLibrary {
        &mut self.shaders
    }

    pub fn set_clear_color(&mut self, color: Color) {
        self.clear_color = color;
    }

    pub fn clear_color(&self) -> Color {
        self.clear_color
    }

    /// Fixe la matrice vue-projection de la frame (fournie par la caméra).
    pub fn set_view_projection(&mut self, view_projection: Mat4) {
        self.view_projection = view_projection;
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.surface_size = (width, height);
        }
        self.backend.resize(width, height);
    }

    /// Dernière taille de surface connue (largeur, hauteur) en pixels.
    pub fn surface_size(&self) -> (u32, u32) {
        self.surface_size
    }

    /// Crée un pipeline graphique : résout la référence shader via la
    /// bibliothèque, puis délègue au backend. Retourne un handle opaque.
    pub fn create_pipeline(
        &mut self,
        descriptor: &PipelineDescriptor,
    ) -> ChaosResult<PipelineHandle> {
        let shader = match &descriptor.shader {
            ShaderRef::Named(name) => self.shaders.get(name).ok_or_else(|| {
                ChaosError::Graphics(format!("shader '{name}' not found in the library"))
            })?,
            ShaderRef::Inline(source) => source,
        };
        self.backend.create_pipeline(descriptor, shader)
    }

    /// Crée un buffer GPU (données uploadées à la création).
    pub fn create_buffer(&mut self, descriptor: &BufferDescriptor) -> ChaosResult<BufferHandle> {
        self.backend.create_buffer(descriptor)
    }

    /// Détruit un buffer GPU ; un handle périmé est une erreur explicite.
    pub fn destroy_buffer(&mut self, handle: BufferHandle) -> ChaosResult<()> {
        self.backend.destroy_buffer(handle)
    }

    /// Crée une texture GPU : applique la validation du descripteur —
    /// une erreur explicite avant tout appel GPU — puis délègue au backend.
    pub fn create_texture(&mut self, descriptor: &TextureDescriptor) -> ChaosResult<TextureHandle> {
        descriptor.validate()?;
        self.backend.create_texture(descriptor)
    }

    /// Détruit une texture GPU ; un handle périmé est une erreur explicite.
    /// Toute entrée du cache pointant vers ce handle est évincée — un
    /// `get_or_create_texture` ultérieur recréera proprement.
    pub fn destroy_texture(&mut self, handle: TextureHandle) -> ChaosResult<()> {
        self.texture_cache.retain(|_, cached| *cached != handle);
        self.backend.destroy_texture(handle)
    }

    /// Cache de textures par clé logique — la clé est le `label` du
    /// descripteur (le futur chemin d'asset). Hit → handle existant ; miss →
    /// création (validation incluse) et insertion. Contrat V1 : la clé fait
    /// foi, pas le contenu — deux descripteurs différents sous le même label
    /// renvoient la première texture créée. `create_texture` reste le chemin
    /// brut qui crée toujours ; le refcount et l'éviction mémoire viendront
    /// avec la gestion mémoire GPU.
    pub fn get_or_create_texture(
        &mut self,
        descriptor: &TextureDescriptor,
    ) -> ChaosResult<TextureHandle> {
        if let Some(handle) = self.texture_cache.get(&descriptor.label) {
            debug!("texture cache hit '{}' ({handle:?})", descriptor.label);
            return Ok(*handle);
        }
        let handle = self.create_texture(descriptor)?;
        self.texture_cache.insert(descriptor.label.clone(), handle);
        Ok(handle)
    }

    /// Crée un sampler GPU — la manière de lire une texture, indépendante
    /// de la texture elle-même.
    pub fn create_sampler(&mut self, descriptor: &SamplerDescriptor) -> ChaosResult<SamplerHandle> {
        self.backend.create_sampler(descriptor)
    }

    /// Détruit un sampler GPU ; un handle périmé est une erreur explicite.
    pub fn destroy_sampler(&mut self, handle: SamplerHandle) -> ChaosResult<()> {
        self.backend.destroy_sampler(handle)
    }

    /// Crée un material — le concept de surface du moteur : le pipeline qui
    /// dessine, la couleur de base, et la texture/le sampler optionnels
    /// (fallbacks builtin : texture blanche 1×1, sampler Linear+Repeat).
    pub fn create_material(
        &mut self,
        descriptor: &MaterialDescriptor,
    ) -> ChaosResult<MaterialHandle> {
        let texture = match descriptor.texture {
            Some(texture) => texture,
            None => self.fallback_texture()?,
        };
        let sampler = match descriptor.sampler {
            Some(sampler) => sampler,
            None => self.fallback_sampler()?,
        };
        let binding = self
            .backend
            .create_material_binding(&MaterialBindingDescriptor::new(
                descriptor.label.clone(),
                texture,
                sampler,
                descriptor.base_color,
            ))?;
        let record = MaterialRecord {
            pipeline: descriptor.pipeline,
            binding,
        };
        let pool_handle = self
            .materials
            .insert(record)
            .ok_or_else(|| ChaosError::Graphics(String::from("material pool capacity exceeded")))?;
        let handle = MaterialHandle {
            index: pool_handle.index,
            generation: pool_handle.generation,
        };
        debug!("material '{}' created ({handle:?})", descriptor.label);
        Ok(handle)
    }

    /// Détruit un material et le binding GPU qu'il possède ; la texture et
    /// le sampler référencés — partageables — ne sont pas touchés. Un handle
    /// périmé est une erreur explicite.
    pub fn destroy_material(&mut self, handle: MaterialHandle) -> ChaosResult<()> {
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        let Some(record) = self.materials.remove(pool_handle) else {
            return Err(ChaosError::Graphics(String::from(
                "material handle is stale or already destroyed",
            )));
        };
        let result = self.backend.destroy_material_binding(record.binding);
        debug!("material released ({handle:?})");
        result
    }

    /// Texture de repli builtin (`chaos.white`, 1×1) — servie par le cache :
    /// créée au premier material sans texture, partagée ensuite, et recréée
    /// automatiquement si quelqu'un la détruit (le cache s'auto-répare).
    fn fallback_texture(&mut self) -> ChaosResult<TextureHandle> {
        self.get_or_create_texture(&TextureDescriptor::sampled(
            "chaos.white",
            1,
            1,
            TextureFormat::Rgba8Unorm,
            vec![255, 255, 255, 255],
        ))
    }

    /// Sampler de repli builtin (`chaos.default_sampler`, Linear + Repeat).
    fn fallback_sampler(&mut self) -> ChaosResult<SamplerHandle> {
        if let Some(sampler) = self.fallback_sampler {
            return Ok(sampler);
        }
        let sampler = self.create_sampler(&SamplerDescriptor::new("chaos.default_sampler"))?;
        self.fallback_sampler = Some(sampler);
        Ok(sampler)
    }

    /// Crée un mesh à sommets colorés : téléverse la géométrie (vertex +
    /// index buffers) et l'enregistre comme ressource de rendu. Le mesh
    /// possède ses buffers.
    pub fn create_mesh(&mut self, label: &str, geometry: &Geometry) -> ChaosResult<MeshHandle> {
        let index_bytes = geometry.is_indexed().then(|| geometry.index_bytes());
        self.register_mesh(
            label,
            geometry.vertex_bytes(),
            index_bytes,
            geometry.element_count(),
            ColorVertex::layout(),
        )
    }

    /// Crée un mesh à sommets texturés (position + UV) — même cycle de vie
    /// que `create_mesh`, layout `TexturedVertex`.
    pub fn create_textured_mesh(
        &mut self,
        label: &str,
        geometry: &TexturedGeometry,
    ) -> ChaosResult<MeshHandle> {
        let index_bytes = geometry.is_indexed().then(|| geometry.index_bytes());
        self.register_mesh(
            label,
            geometry.vertex_bytes(),
            index_bytes,
            geometry.element_count(),
            TexturedVertex::layout(),
        )
    }

    fn register_mesh(
        &mut self,
        label: &str,
        vertex_bytes: Vec<u8>,
        index_bytes: Option<Vec<u8>>,
        element_count: u32,
        vertex_layout: VertexLayout,
    ) -> ChaosResult<MeshHandle> {
        let vertex_buffer = self
            .backend
            .create_buffer(&BufferDescriptor::vertex(label, vertex_bytes))?;
        let index_buffer = match index_bytes {
            Some(bytes) => Some(
                self.backend
                    .create_buffer(&BufferDescriptor::index(format!("{label}.indices"), bytes))?,
            ),
            None => None,
        };
        let record = MeshRecord {
            vertex_buffer,
            index_buffer,
            element_count,
            vertex_layout,
        };
        let stride = record.vertex_layout.stride;
        let pool_handle = self
            .meshes
            .insert(record)
            .ok_or_else(|| ChaosError::Graphics(String::from("mesh pool capacity exceeded")))?;
        let handle = MeshHandle {
            index: pool_handle.index,
            generation: pool_handle.generation,
        };
        debug!("mesh '{label}' created ({element_count} elements, stride {stride}, {handle:?})");
        Ok(handle)
    }

    /// Détruit un mesh et les buffers qu'il possède ; un handle périmé est
    /// une erreur explicite.
    pub fn destroy_mesh(&mut self, handle: MeshHandle) -> ChaosResult<()> {
        let pool_handle = PoolHandle {
            index: handle.index,
            generation: handle.generation,
        };
        let Some(record) = self.meshes.remove(pool_handle) else {
            return Err(ChaosError::Graphics(String::from(
                "mesh handle is stale or already destroyed",
            )));
        };
        let mut first_error = self.backend.destroy_buffer(record.vertex_buffer).err();
        if let Some(index_buffer) = record.index_buffer
            && let Err(destroy_error) = self.backend.destroy_buffer(index_buffer)
            && first_error.is_none()
        {
            first_error = Some(destroy_error);
        }
        debug!("mesh released ({handle:?})");
        match first_error {
            Some(destroy_error) => Err(destroy_error),
            None => Ok(()),
        }
    }

    /// Soumet un ordre de dessin à la RenderQueue de la frame de simulation
    /// courante.
    pub fn queue_draw(&mut self, command: DrawCommand) {
        self.queue.submit(command);
    }

    /// Vide la RenderQueue — appelée par le moteur au début de chaque
    /// frame de simulation. Les draws survivent ainsi aux présentations
    /// multiples entre deux updates (rafales de redraw du resize interactif).
    pub fn clear_draws(&mut self) {
        self.queue.clear();
    }

    /// Le nombre de draws soumis pour la frame de simulation courante —
    /// la jauge des metrics de santé.
    pub fn draw_count(&self) -> usize {
        self.queue.len()
    }

    /// Construit le plan de la frame courante : les draws sont pris dans
    /// l'ordre de rendu de la RenderQueue, leurs materials résolus en
    /// (pipeline, binding) et leurs meshes en buffers (une ressource
    /// détruite entre-temps est écartée avec un warn), puis le plan est
    /// exécuté par le backend. Les draws restent en place jusqu'au prochain
    /// `clear_draws` du moteur.
    pub fn render_frame(&mut self) -> ChaosResult<FrameOutcome> {
        let commands = self.queue.ordered();
        let mut draws = Vec::with_capacity(commands.len());
        for command in commands {
            let material_handle = PoolHandle {
                index: command.material.index,
                generation: command.material.generation,
            };
            let Some(material) = self.materials.get(material_handle) else {
                warn!("draw dropped: stale material {:?}", command.material);
                continue;
            };
            let pool_handle = PoolHandle {
                index: command.mesh.index,
                generation: command.mesh.generation,
            };
            let Some(record) = self.meshes.get(pool_handle) else {
                warn!("draw dropped: stale mesh {:?}", command.mesh);
                continue;
            };
            draws.push(FrameDraw {
                pipeline: material.pipeline,
                vertex_buffer: Some(record.vertex_buffer),
                index_buffer: record.index_buffer,
                element_count: record.element_count,
                model: command.transform.matrix(),
                binding: Some(material.binding),
            });
        }
        let plan = FramePlan {
            clear_color: self.clear_color,
            view_projection: self.view_projection,
            draws,
        };
        self.backend.render(&plan)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use chaos_core::Transform;
    use chaos_core::math::Vec3;

    use crate::frame::FrameSkipReason;
    use crate::resources::{
        MaterialBindingHandle, SamplerAddressMode, SamplerFilter, ShaderSource,
    };
    use crate::shaders::builtin;

    use super::*;

    #[derive(Clone, Default)]
    struct Journal(Arc<Mutex<Vec<String>>>);

    impl Journal {
        fn push(&self, entry: String) {
            self.0.lock().unwrap().push(entry);
        }

        fn entries(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    struct MockBackend {
        journal: Journal,
        outcome: FrameOutcome,
        pipelines_created: u32,
        buffers_created: u32,
        textures_created: u32,
        samplers_created: u32,
        material_bindings_created: u32,
    }

    impl GraphicsBackend for MockBackend {
        fn description(&self) -> String {
            String::from("mock backend")
        }

        fn resize(&mut self, width: u32, height: u32) {
            self.journal.push(format!("resize {width}x{height}"));
        }

        fn create_pipeline(
            &mut self,
            descriptor: &PipelineDescriptor,
            shader: &ShaderSource,
        ) -> ChaosResult<PipelineHandle> {
            let ShaderSource::Wgsl(code) = shader;
            self.journal.push(format!(
                "create_pipeline {} code={}",
                descriptor.label,
                &code[..code.len().min(24)]
            ));
            let handle = PipelineHandle(self.pipelines_created);
            self.pipelines_created += 1;
            Ok(handle)
        }

        fn create_buffer(&mut self, descriptor: &BufferDescriptor) -> ChaosResult<BufferHandle> {
            self.journal.push(format!(
                "create_buffer {} kind={:?} bytes={}",
                descriptor.label,
                descriptor.kind,
                descriptor.contents.len()
            ));
            let handle = BufferHandle {
                index: self.buffers_created,
                generation: 0,
            };
            self.buffers_created += 1;
            Ok(handle)
        }

        fn destroy_buffer(&mut self, handle: BufferHandle) -> ChaosResult<()> {
            self.journal
                .push(format!("destroy_buffer index={}", handle.index));
            Ok(())
        }

        fn create_texture(&mut self, descriptor: &TextureDescriptor) -> ChaosResult<TextureHandle> {
            self.journal.push(format!(
                "create_texture {} {}x{} format={:?} usage={:?} bytes={}",
                descriptor.label,
                descriptor.width,
                descriptor.height,
                descriptor.format,
                descriptor.usage,
                descriptor.pixels.len()
            ));
            let handle = TextureHandle {
                index: self.textures_created,
                generation: 0,
            };
            self.textures_created += 1;
            Ok(handle)
        }

        fn destroy_texture(&mut self, handle: TextureHandle) -> ChaosResult<()> {
            self.journal
                .push(format!("destroy_texture index={}", handle.index));
            Ok(())
        }

        fn create_sampler(&mut self, descriptor: &SamplerDescriptor) -> ChaosResult<SamplerHandle> {
            self.journal.push(format!(
                "create_sampler {} filter={:?} address={:?}",
                descriptor.label, descriptor.filter, descriptor.address_mode
            ));
            let handle = SamplerHandle {
                index: self.samplers_created,
                generation: 0,
            };
            self.samplers_created += 1;
            Ok(handle)
        }

        fn destroy_sampler(&mut self, handle: SamplerHandle) -> ChaosResult<()> {
            self.journal
                .push(format!("destroy_sampler index={}", handle.index));
            Ok(())
        }

        fn create_material_binding(
            &mut self,
            descriptor: &MaterialBindingDescriptor,
        ) -> ChaosResult<MaterialBindingHandle> {
            self.journal.push(format!(
                "create_material_binding {} texture={} sampler={} color=({}, {}, {}, {})",
                descriptor.label,
                descriptor.texture.index,
                descriptor.sampler.index,
                descriptor.base_color.r,
                descriptor.base_color.g,
                descriptor.base_color.b,
                descriptor.base_color.a
            ));
            let handle = MaterialBindingHandle {
                index: self.material_bindings_created,
                generation: 0,
            };
            self.material_bindings_created += 1;
            Ok(handle)
        }

        fn destroy_material_binding(&mut self, handle: MaterialBindingHandle) -> ChaosResult<()> {
            self.journal
                .push(format!("destroy_material_binding index={}", handle.index));
            Ok(())
        }

        fn render(&mut self, plan: &FramePlan) -> ChaosResult<FrameOutcome> {
            let color = plan.clear_color;
            let vp = plan.view_projection.w_axis;
            let draws: Vec<String> = plan
                .draws
                .iter()
                .map(|draw| {
                    let model = draw.model.w_axis;
                    format!(
                        "({}, {:?}, {:?}, {}, b={:?}, m=[{}, {}, {}])",
                        draw.pipeline.0,
                        draw.vertex_buffer.map(|buffer| buffer.index),
                        draw.index_buffer.map(|buffer| buffer.index),
                        draw.element_count,
                        draw.binding.map(|binding| binding.index),
                        model.x,
                        model.y,
                        model.z
                    )
                })
                .collect();
            self.journal.push(format!(
                "render r={} g={} b={} a={} vp=[{}, {}, {}] draws=[{}]",
                color.r,
                color.g,
                color.b,
                color.a,
                vp.x,
                vp.y,
                vp.z,
                draws.join(", ")
            ));
            Ok(self.outcome)
        }
    }

    fn mock_renderer_with(outcome: FrameOutcome) -> (Renderer, Journal) {
        let journal = Journal::default();
        let renderer = Renderer::with_backend(Box::new(MockBackend {
            journal: journal.clone(),
            outcome,
            pipelines_created: 0,
            buffers_created: 0,
            textures_created: 0,
            samplers_created: 0,
            material_bindings_created: 0,
        }));
        (renderer, journal)
    }

    fn mock_renderer() -> (Renderer, Journal) {
        mock_renderer_with(FrameOutcome::Rendered)
    }

    fn inline_descriptor(label: &str) -> PipelineDescriptor {
        PipelineDescriptor::new(label, ShaderSource::Wgsl(String::from("inline-code")))
    }

    fn triangle() -> Geometry {
        Geometry::triangle(
            [0.0, 0.0, 0.0],
            1.0,
            [Color::WHITE, Color::WHITE, Color::WHITE],
        )
    }

    fn quad() -> Geometry {
        Geometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, Color::WHITE)
    }

    fn cube() -> Geometry {
        Geometry::cube([0.0, 0.0, 0.0], 1.0, [Color::WHITE; 6])
    }

    fn plain_material(renderer: &mut Renderer, label: &str) -> MaterialHandle {
        let pipeline = renderer.create_pipeline(&inline_descriptor(label)).unwrap();
        renderer
            .create_material(&MaterialDescriptor::new(label, pipeline))
            .unwrap()
    }

    #[test]
    fn frame_plan_carries_current_clear_color() {
        let (mut renderer, journal) = mock_renderer();
        renderer.render_frame().unwrap();
        assert_eq!(
            journal.entries(),
            vec!["render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[]"]
        );
    }

    #[test]
    fn set_clear_color_changes_the_plan() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_clear_color(Color::rgb(1.0, 0.5, 0.25));
        renderer.render_frame().unwrap();
        assert_eq!(
            journal.entries(),
            vec!["render r=1 g=0.5 b=0.25 a=1 vp=[0, 0, 0] draws=[]"]
        );
        assert_eq!(renderer.clear_color(), Color::rgb(1.0, 0.5, 0.25));
    }

    #[test]
    fn backend_outcome_is_propagated() {
        let skipped = FrameOutcome::Skipped(FrameSkipReason::SurfaceUnavailable);
        let (mut renderer, _journal) = mock_renderer_with(skipped);
        assert_eq!(renderer.render_frame().unwrap(), skipped);
    }

    #[test]
    fn resize_is_forwarded_to_backend() {
        let (mut renderer, journal) = mock_renderer();
        renderer.resize(1920, 1080);
        assert_eq!(journal.entries(), vec!["resize 1920x1080"]);
        assert_eq!(renderer.surface_size(), (1920, 1080));
        renderer.resize(0, 0);
        assert_eq!(renderer.surface_size(), (1920, 1080));
    }

    #[test]
    fn description_delegates_to_backend() {
        let (renderer, _journal) = mock_renderer();
        assert_eq!(renderer.description(), "mock backend");
    }

    #[test]
    fn create_pipeline_returns_increasing_handles() {
        let (mut renderer, _journal) = mock_renderer();
        let first = renderer.create_pipeline(&inline_descriptor("a")).unwrap();
        let second = renderer.create_pipeline(&inline_descriptor("b")).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn inline_shader_reaches_the_backend() {
        let (mut renderer, journal) = mock_renderer();
        renderer.create_pipeline(&inline_descriptor("a")).unwrap();
        assert_eq!(
            journal.entries(),
            vec!["create_pipeline a code=inline-code"]
        );
    }

    #[test]
    fn named_shader_resolves_through_the_library() {
        let (mut renderer, journal) = mock_renderer();
        renderer.shaders_mut().register(
            "game.custom",
            ShaderSource::Wgsl(String::from("custom-code")),
        );
        let descriptor = PipelineDescriptor::new("t", "game.custom");
        renderer.create_pipeline(&descriptor).unwrap();
        assert_eq!(
            journal.entries(),
            vec!["create_pipeline t code=custom-code"]
        );
    }

    #[test]
    fn unknown_named_shader_is_a_comprehensible_error() {
        let (mut renderer, journal) = mock_renderer();
        let descriptor = PipelineDescriptor::new("t", "missing.shader");
        let error = renderer.create_pipeline(&descriptor).unwrap_err();
        assert!(error.to_string().contains("missing.shader"));
        assert!(journal.entries().is_empty());
    }

    #[test]
    fn builtin_vertex_color_is_available() {
        let (renderer, _journal) = mock_renderer();
        assert!(renderer.shaders().contains(builtin::VERTEX_COLOR));
    }

    #[test]
    fn create_buffer_forwards_descriptor_and_returns_distinct_handles() {
        let (mut renderer, journal) = mock_renderer();
        let first = renderer
            .create_buffer(&BufferDescriptor::vertex("tri", vec![0, 1, 2, 3]))
            .unwrap();
        let second = renderer
            .create_buffer(&BufferDescriptor::index("idx", vec![0, 1]))
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(
            journal.entries(),
            vec![
                "create_buffer tri kind=Vertex bytes=4",
                "create_buffer idx kind=Index bytes=2"
            ]
        );
    }

    #[test]
    fn destroy_buffer_forwards_the_handle() {
        let (mut renderer, journal) = mock_renderer();
        let handle = renderer
            .create_buffer(&BufferDescriptor::vertex("tri", Vec::new()))
            .unwrap();
        renderer.destroy_buffer(handle).unwrap();
        assert_eq!(
            journal.entries(),
            vec![
                "create_buffer tri kind=Vertex bytes=0",
                "destroy_buffer index=0"
            ]
        );
    }

    #[test]
    fn create_texture_forwards_descriptor_and_returns_distinct_handles() {
        let (mut renderer, journal) = mock_renderer();
        let first = renderer
            .create_texture(&TextureDescriptor::sampled(
                "albedo",
                2,
                2,
                TextureFormat::Rgba8UnormSrgb,
                vec![255; 16],
            ))
            .unwrap();
        let second = renderer
            .create_texture(&TextureDescriptor::render_target(
                "offscreen",
                4,
                4,
                TextureFormat::Rgba8Unorm,
            ))
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(
            journal.entries(),
            vec![
                "create_texture albedo 2x2 format=Rgba8UnormSrgb usage=Sampled bytes=16",
                "create_texture offscreen 4x4 format=Rgba8Unorm usage=RenderTarget bytes=0"
            ]
        );
    }

    #[test]
    fn destroy_texture_forwards_the_handle() {
        let (mut renderer, journal) = mock_renderer();
        let handle = renderer
            .create_texture(&TextureDescriptor::sampled(
                "mask",
                1,
                1,
                TextureFormat::R8Unorm,
                vec![128],
            ))
            .unwrap();
        renderer.destroy_texture(handle).unwrap();
        assert_eq!(
            journal.entries(),
            vec![
                "create_texture mask 1x1 format=R8Unorm usage=Sampled bytes=1",
                "destroy_texture index=0"
            ]
        );
    }

    #[test]
    fn texture_with_wrong_pixel_size_is_rejected_before_the_backend() {
        let (mut renderer, journal) = mock_renderer();
        let error = renderer
            .create_texture(&TextureDescriptor::sampled(
                "bad",
                2,
                2,
                TextureFormat::Rgba8Unorm,
                vec![0; 3],
            ))
            .unwrap_err();
        assert!(error.to_string().contains("16 bytes"));
        assert!(error.to_string().contains("got 3"));
        assert!(journal.entries().is_empty());
    }

    #[test]
    fn render_target_with_initial_pixels_is_rejected() {
        let (mut renderer, journal) = mock_renderer();
        let mut descriptor =
            TextureDescriptor::render_target("rt", 2, 2, TextureFormat::Rgba8Unorm);
        descriptor.pixels = vec![0; 16];
        let error = renderer.create_texture(&descriptor).unwrap_err();
        assert!(error.to_string().contains("render target"));
        assert!(journal.entries().is_empty());
    }

    #[test]
    fn zero_sized_texture_is_rejected() {
        let (mut renderer, journal) = mock_renderer();
        let error = renderer
            .create_texture(&TextureDescriptor::sampled(
                "empty",
                0,
                4,
                TextureFormat::R8Unorm,
                Vec::new(),
            ))
            .unwrap_err();
        assert!(error.to_string().contains("zero dimensions"));
        assert!(journal.entries().is_empty());
    }

    #[test]
    fn create_sampler_forwards_descriptor_and_returns_distinct_handles() {
        let (mut renderer, journal) = mock_renderer();
        let first = renderer
            .create_sampler(&SamplerDescriptor::new("linear"))
            .unwrap();
        let second = renderer
            .create_sampler(
                &SamplerDescriptor::new("pixel")
                    .with_filter(SamplerFilter::Nearest)
                    .with_address_mode(SamplerAddressMode::ClampToEdge),
            )
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(
            journal.entries(),
            vec![
                "create_sampler linear filter=Linear address=Repeat",
                "create_sampler pixel filter=Nearest address=ClampToEdge"
            ]
        );
    }

    #[test]
    fn destroy_sampler_forwards_the_handle() {
        let (mut renderer, journal) = mock_renderer();
        let handle = renderer
            .create_sampler(&SamplerDescriptor::new("s"))
            .unwrap();
        renderer.destroy_sampler(handle).unwrap();
        assert_eq!(
            journal.entries(),
            vec![
                "create_sampler s filter=Linear address=Repeat",
                "destroy_sampler index=0"
            ]
        );
    }

    fn texture_and_sampler(renderer: &mut Renderer) -> (TextureHandle, SamplerHandle) {
        let texture = renderer
            .create_texture(&TextureDescriptor::sampled(
                "albedo",
                1,
                1,
                TextureFormat::R8Unorm,
                vec![255],
            ))
            .unwrap();
        let sampler = renderer
            .create_sampler(&SamplerDescriptor::new("s"))
            .unwrap();
        (texture, sampler)
    }

    #[test]
    fn get_or_create_texture_deduplicates_by_label() {
        let (mut renderer, journal) = mock_renderer();
        let descriptor =
            TextureDescriptor::sampled("shared", 1, 1, TextureFormat::R8Unorm, vec![255]);
        let first = renderer.get_or_create_texture(&descriptor).unwrap();
        let second = renderer.get_or_create_texture(&descriptor).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            journal.entries(),
            vec!["create_texture shared 1x1 format=R8Unorm usage=Sampled bytes=1"]
        );
    }

    #[test]
    fn get_or_create_texture_recreates_after_destroy() {
        let (mut renderer, journal) = mock_renderer();
        let descriptor =
            TextureDescriptor::sampled("shared", 1, 1, TextureFormat::R8Unorm, vec![255]);
        let first = renderer.get_or_create_texture(&descriptor).unwrap();
        renderer.destroy_texture(first).unwrap();
        let second = renderer.get_or_create_texture(&descriptor).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            journal.entries(),
            vec![
                "create_texture shared 1x1 format=R8Unorm usage=Sampled bytes=1",
                "destroy_texture index=0",
                "create_texture shared 1x1 format=R8Unorm usage=Sampled bytes=1"
            ]
        );
    }

    #[test]
    fn distinct_labels_create_distinct_textures() {
        let (mut renderer, _journal) = mock_renderer();
        let first = renderer
            .get_or_create_texture(&TextureDescriptor::sampled(
                "a",
                1,
                1,
                TextureFormat::R8Unorm,
                vec![255],
            ))
            .unwrap();
        let second = renderer
            .get_or_create_texture(&TextureDescriptor::sampled(
                "b",
                1,
                1,
                TextureFormat::R8Unorm,
                vec![255],
            ))
            .unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn destroyed_fallback_texture_self_heals() {
        let (mut renderer, journal) = mock_renderer();
        let pipeline = renderer.create_pipeline(&inline_descriptor("p")).unwrap();
        renderer
            .create_material(&MaterialDescriptor::new("a", pipeline))
            .unwrap();
        let fallback = TextureHandle {
            index: 0,
            generation: 0,
        };
        renderer.destroy_texture(fallback).unwrap();
        renderer
            .create_material(&MaterialDescriptor::new("b", pipeline))
            .unwrap();
        let entries = journal.entries();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.contains("create_texture chaos.white"))
                .count(),
            2
        );
    }

    #[test]
    fn create_material_uses_builtin_fallbacks_once() {
        let (mut renderer, journal) = mock_renderer();
        let pipeline = renderer.create_pipeline(&inline_descriptor("p")).unwrap();
        renderer
            .create_material(&MaterialDescriptor::new("a", pipeline))
            .unwrap();
        renderer
            .create_material(&MaterialDescriptor::new("b", pipeline))
            .unwrap();
        let entries = journal.entries();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.contains("chaos.white"))
                .count(),
            1
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.contains("chaos.default_sampler"))
                .count(),
            1
        );
        assert_eq!(
            entries[entries.len() - 1],
            "create_material_binding b texture=0 sampler=0 color=(1, 1, 1, 1)"
        );
    }

    #[test]
    fn create_material_forwards_texture_sampler_and_color() {
        let (mut renderer, journal) = mock_renderer();
        let pipeline = renderer.create_pipeline(&inline_descriptor("p")).unwrap();
        let (texture, sampler) = texture_and_sampler(&mut renderer);
        renderer
            .create_material(
                &MaterialDescriptor::new("m", pipeline)
                    .with_base_color(Color::rgb(0.5, 0.25, 1.0))
                    .with_texture(texture)
                    .with_sampler(sampler),
            )
            .unwrap();
        let entries = journal.entries();
        assert_eq!(
            entries[entries.len() - 1],
            "create_material_binding m texture=0 sampler=0 color=(0.5, 0.25, 1, 1)"
        );
    }

    #[test]
    fn destroy_material_destroys_its_binding() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        renderer.destroy_material(material).unwrap();
        let entries = journal.entries();
        assert_eq!(
            entries[entries.len() - 1],
            "destroy_material_binding index=0"
        );
        let error = renderer.destroy_material(material).unwrap_err();
        assert!(error.to_string().contains("stale"));
    }

    #[test]
    fn the_renderer_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Renderer>();
    }

    #[test]
    fn draw_count_reports_the_submitted_frame() {
        let (mut renderer, _journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        assert_eq!(renderer.draw_count(), 0);
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        assert_eq!(renderer.draw_count(), 2);
        renderer.clear_draws();
        assert_eq!(renderer.draw_count(), 0);
    }

    #[test]
    fn stale_material_draw_is_dropped_from_the_plan() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.destroy_material(material).unwrap();
        renderer.render_frame().unwrap();
        let entries = journal.entries();
        assert_eq!(
            entries[entries.len() - 1],
            "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[]"
        );
    }

    #[test]
    fn create_mesh_uploads_the_right_buffers() {
        let (mut renderer, journal) = mock_renderer();
        let first = renderer.create_mesh("tri", &triangle()).unwrap();
        let second = renderer.create_mesh("quad", &quad()).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            journal.entries(),
            vec![
                "create_buffer tri kind=Vertex bytes=72",
                "create_buffer quad kind=Vertex bytes=96",
                "create_buffer quad.indices kind=Index bytes=12"
            ]
        );
    }

    #[test]
    fn create_textured_mesh_uploads_the_uv_vertices() {
        let (mut renderer, journal) = mock_renderer();
        let quad = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 4.0);
        renderer.create_textured_mesh("floor", &quad).unwrap();
        assert_eq!(
            journal.entries(),
            vec![
                "create_buffer floor kind=Vertex bytes=80",
                "create_buffer floor.indices kind=Index bytes=12"
            ]
        );
    }

    #[test]
    fn mesh_draws_resolve_into_the_plan_then_reset() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        let tri = renderer.create_mesh("tri", &triangle()).unwrap();
        let quad = renderer.create_mesh("quad", &quad()).unwrap();
        renderer.queue_draw(DrawCommand {
            mesh: tri,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.queue_draw(DrawCommand {
            mesh: quad,
            material,
            transform: Transform::from_translation(Vec3::new(5.0, 0.0, 0.0)),
        });
        renderer.render_frame().unwrap();
        renderer.render_frame().unwrap();
        renderer.clear_draws();
        renderer.render_frame().unwrap();
        let entries = journal.entries();
        let full_plan = "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[(0, Some(0), None, 3, b=Some(0), m=[0, 0, 0]), (0, Some(1), Some(2), 6, b=Some(0), m=[5, 0, 0])]";
        assert_eq!(entries[entries.len() - 3], full_plan);
        assert_eq!(entries[entries.len() - 2], full_plan);
        assert_eq!(
            entries[entries.len() - 1],
            "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[]"
        );
    }

    #[test]
    fn interleaved_materials_are_grouped_in_the_plan() {
        let (mut renderer, journal) = mock_renderer();
        let first = plain_material(&mut renderer, "a");
        let second = plain_material(&mut renderer, "b");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        let submissions = [(second, 1.0), (first, 2.0), (second, 3.0), (first, 4.0)];
        for (material, x) in submissions {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::from_translation(Vec3::new(x, 0.0, 0.0)),
            });
        }
        renderer.render_frame().unwrap();
        let entries = journal.entries();
        assert_eq!(
            entries[entries.len() - 1],
            "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[\
             (0, Some(0), None, 3, b=Some(0), m=[2, 0, 0]), \
             (0, Some(0), None, 3, b=Some(0), m=[4, 0, 0]), \
             (1, Some(0), None, 3, b=Some(1), m=[1, 0, 0]), \
             (1, Some(0), None, 3, b=Some(1), m=[3, 0, 0])]"
        );
    }

    #[test]
    fn shared_mesh_draws_with_distinct_transforms() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("cube", &cube()).unwrap();
        for x in [-2.0, 0.0, 2.0] {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::from_translation(Vec3::new(x, 0.0, 0.0)),
            });
        }
        renderer.render_frame().unwrap();
        let entries = journal.entries();
        assert_eq!(
            entries[entries.len() - 1],
            "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[\
             (0, Some(0), Some(1), 36, b=Some(0), m=[-2, 0, 0]), \
             (0, Some(0), Some(1), 36, b=Some(0), m=[0, 0, 0]), \
             (0, Some(0), Some(1), 36, b=Some(0), m=[2, 0, 0])]"
        );
    }

    #[test]
    fn many_draws_reach_the_plan_in_submission_order() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("cube", &cube()).unwrap();
        for index in 0u8..16 {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::from_translation(Vec3::new(f32::from(index), 0.0, 0.0)),
            });
        }
        renderer.render_frame().unwrap();
        let entries = journal.entries();
        let plan = entries[entries.len() - 1].clone();
        assert_eq!(plan.matches("m=[").count(), 16);
        let positions: Vec<usize> = (0u8..16)
            .map(|index| {
                plan.find(&format!("m=[{}, 0, 0]", f32::from(index)))
                    .unwrap()
            })
            .collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn destroy_mesh_destroys_its_buffers() {
        let (mut renderer, journal) = mock_renderer();
        let mesh = renderer.create_mesh("quad", &quad()).unwrap();
        renderer.destroy_mesh(mesh).unwrap();
        let entries = journal.entries();
        assert!(entries.contains(&String::from("destroy_buffer index=0")));
        assert!(entries.contains(&String::from("destroy_buffer index=1")));
        let error = renderer.destroy_mesh(mesh).unwrap_err();
        assert!(error.to_string().contains("stale"));
    }

    #[test]
    fn stale_mesh_draw_is_dropped_from_the_plan() {
        let (mut renderer, journal) = mock_renderer();
        let material = plain_material(&mut renderer, "p");
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.destroy_mesh(mesh).unwrap();
        renderer.render_frame().unwrap();
        let entries = journal.entries();
        assert_eq!(
            entries[entries.len() - 1],
            "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[]"
        );
    }

    #[test]
    fn view_projection_travels_in_the_plan() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_view_projection(Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)));
        renderer.render_frame().unwrap();
        assert_eq!(
            journal.entries(),
            vec!["render r=0 g=0 b=0 a=1 vp=[1, 2, 3] draws=[]"]
        );
    }
}
