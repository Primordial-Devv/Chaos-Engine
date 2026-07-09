use chaos_core::{ChaosError, ChaosResult, Color};
use log::{debug, info, warn};

use crate::backend::{GraphicsBackend, create_backend};
use crate::config::RendererConfig;
use crate::frame::{DrawCommand, FrameDraw, FrameOutcome, FramePlan};
use crate::geometry::Geometry;
use crate::mesh::{MeshHandle, MeshRecord};
use crate::pool::{PoolHandle, ResourcePool};
use crate::resources::{
    BufferDescriptor, BufferHandle, ColorVertex, PipelineDescriptor, PipelineHandle, ShaderRef,
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
    clear_color: Color,
    pending_draws: Vec<DrawCommand>,
}

impl Renderer {
    /// Attache le renderer à une cible de présentation et initialise le GPU.
    pub fn attach(
        target: impl SurfaceTarget + 'static,
        config: RendererConfig,
    ) -> ChaosResult<Self> {
        let backend = create_backend(Box::new(target), config)?;
        info!("renderer ready: {}", backend.description());
        Ok(Self::with_backend(backend))
    }

    pub(crate) fn with_backend(backend: Box<dyn GraphicsBackend>) -> Self {
        Self {
            backend,
            shaders: ShaderLibrary::with_builtins(),
            meshes: ResourcePool::new(),
            clear_color: Color::BLACK,
            pending_draws: Vec::new(),
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

    pub fn resize(&mut self, width: u32, height: u32) {
        self.backend.resize(width, height);
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

    /// Crée un mesh : téléverse la géométrie (vertex + index buffers) et
    /// l'enregistre comme ressource de rendu. Le mesh possède ses buffers.
    pub fn create_mesh(&mut self, label: &str, geometry: &Geometry) -> ChaosResult<MeshHandle> {
        let vertex_buffer = self
            .backend
            .create_buffer(&BufferDescriptor::vertex(label, geometry.vertex_bytes()))?;
        let index_buffer = if geometry.is_indexed() {
            Some(self.backend.create_buffer(&BufferDescriptor::index(
                format!("{label}.indices"),
                geometry.index_bytes(),
            ))?)
        } else {
            None
        };
        let record = MeshRecord {
            vertex_buffer,
            index_buffer,
            element_count: geometry.element_count(),
            vertex_layout: ColorVertex::layout(),
        };
        let element_count = record.element_count;
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

    /// Enregistre un ordre de dessin pour la prochaine frame.
    pub fn queue_draw(&mut self, command: DrawCommand) {
        self.pending_draws.push(command);
    }

    /// Construit le plan de la frame courante — les meshes des draws sont
    /// résolus en buffers ici (un mesh détruit entre-temps est écarté avec
    /// un warn) — puis le fait exécuter au backend. La file repart vide.
    pub fn render_frame(&mut self) -> ChaosResult<FrameOutcome> {
        let mut draws = Vec::with_capacity(self.pending_draws.len());
        for command in std::mem::take(&mut self.pending_draws) {
            let pool_handle = PoolHandle {
                index: command.mesh.index,
                generation: command.mesh.generation,
            };
            let Some(record) = self.meshes.get(pool_handle) else {
                warn!("draw dropped: stale mesh {:?}", command.mesh);
                continue;
            };
            draws.push(FrameDraw {
                pipeline: command.pipeline,
                vertex_buffer: Some(record.vertex_buffer),
                index_buffer: record.index_buffer,
                element_count: record.element_count,
            });
        }
        let plan = FramePlan {
            clear_color: self.clear_color,
            draws,
        };
        self.backend.render(&plan)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::frame::FrameSkipReason;
    use crate::resources::ShaderSource;
    use crate::shaders::builtin;

    use super::*;

    #[derive(Clone, Default)]
    struct Journal(Rc<RefCell<Vec<String>>>);

    impl Journal {
        fn push(&self, entry: String) {
            self.0.borrow_mut().push(entry);
        }

        fn entries(&self) -> Vec<String> {
            self.0.borrow().clone()
        }
    }

    struct MockBackend {
        journal: Journal,
        outcome: FrameOutcome,
        pipelines_created: u32,
        buffers_created: u32,
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

        fn render(&mut self, plan: &FramePlan) -> ChaosResult<FrameOutcome> {
            let color = plan.clear_color;
            let draws: Vec<(u32, Option<u32>, Option<u32>, u32)> = plan
                .draws
                .iter()
                .map(|draw| {
                    (
                        draw.pipeline.0,
                        draw.vertex_buffer.map(|buffer| buffer.index),
                        draw.index_buffer.map(|buffer| buffer.index),
                        draw.element_count,
                    )
                })
                .collect();
            self.journal.push(format!(
                "render r={} g={} b={} a={} draws={draws:?}",
                color.r, color.g, color.b, color.a
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

    #[test]
    fn frame_plan_carries_current_clear_color() {
        let (mut renderer, journal) = mock_renderer();
        renderer.render_frame().unwrap();
        assert_eq!(journal.entries(), vec!["render r=0 g=0 b=0 a=1 draws=[]"]);
    }

    #[test]
    fn set_clear_color_changes_the_plan() {
        let (mut renderer, journal) = mock_renderer();
        renderer.set_clear_color(Color::rgb(1.0, 0.5, 0.25));
        renderer.render_frame().unwrap();
        assert_eq!(
            journal.entries(),
            vec!["render r=1 g=0.5 b=0.25 a=1 draws=[]"]
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
    fn mesh_draws_resolve_into_the_plan_then_reset() {
        let (mut renderer, journal) = mock_renderer();
        let pipeline = renderer.create_pipeline(&inline_descriptor("p")).unwrap();
        let tri = renderer.create_mesh("tri", &triangle()).unwrap();
        let quad = renderer.create_mesh("quad", &quad()).unwrap();
        renderer.queue_draw(DrawCommand {
            pipeline,
            mesh: tri,
        });
        renderer.queue_draw(DrawCommand {
            pipeline,
            mesh: quad,
        });
        renderer.render_frame().unwrap();
        renderer.render_frame().unwrap();
        let entries = journal.entries();
        assert_eq!(
            entries[entries.len() - 2],
            "render r=0 g=0 b=0 a=1 draws=[(0, Some(0), None, 3), (0, Some(1), Some(2), 6)]"
        );
        assert_eq!(
            entries[entries.len() - 1],
            "render r=0 g=0 b=0 a=1 draws=[]"
        );
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
        let pipeline = renderer.create_pipeline(&inline_descriptor("p")).unwrap();
        let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
        renderer.queue_draw(DrawCommand { pipeline, mesh });
        renderer.destroy_mesh(mesh).unwrap();
        renderer.render_frame().unwrap();
        let entries = journal.entries();
        assert_eq!(
            entries[entries.len() - 1],
            "render r=0 g=0 b=0 a=1 draws=[]"
        );
    }
}
