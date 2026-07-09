use chaos_engine::{
    ChaosResult, Color, ColorVertex, DrawCommand, EngineContext, Geometry, MeshHandle,
    PipelineDescriptor, PipelineHandle, Subsystem, shaders,
};

/// Démo de géométrie : un triangle et un quad, construits par les primitives
/// du moteur et téléversés comme meshes. N'utilise que le vocabulaire haut
/// niveau — l'API d'un futur gamemode.
#[derive(Default)]
pub struct GeometryDemo {
    pipeline: Option<PipelineHandle>,
    meshes: Vec<MeshHandle>,
}

impl Subsystem for GeometryDemo {
    fn name(&self) -> &str {
        "geometry_demo"
    }

    fn init(&mut self, context: &mut EngineContext) -> ChaosResult<()> {
        let Some(renderer) = context.renderer_mut() else {
            return Ok(());
        };

        let descriptor = PipelineDescriptor::new("demo.geometry", shaders::builtin::VERTEX_COLOR)
            .with_vertex_layout(ColorVertex::layout());
        let pipeline = renderer.create_pipeline(&descriptor)?;

        let triangle = Geometry::triangle(
            [-0.5, 0.0, 0.0],
            0.9,
            [
                Color::rgb(0.95, 0.25, 0.85),
                Color::rgb(0.45, 0.15, 0.95),
                Color::rgb(0.20, 0.80, 0.95),
            ],
        );
        let quad = Geometry::quad([0.5, 0.0, 0.0], 0.45, 0.8, Color::rgb(0.55, 0.20, 0.90));

        self.meshes = vec![
            renderer.create_mesh("demo.triangle", &triangle)?,
            renderer.create_mesh("demo.quad", &quad)?,
        ];
        self.pipeline = Some(pipeline);
        Ok(())
    }

    fn update(&mut self, context: &mut EngineContext) {
        let Some(pipeline) = self.pipeline else {
            return;
        };
        let Some(renderer) = context.renderer_mut() else {
            return;
        };
        for mesh in &self.meshes {
            renderer.queue_draw(DrawCommand {
                pipeline,
                mesh: *mesh,
            });
        }
    }
}
