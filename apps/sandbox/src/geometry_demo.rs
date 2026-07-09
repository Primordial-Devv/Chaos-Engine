use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

use chaos_engine::{
    Camera, ChaosResult, Color, ColorVertex, CullMode, DrawCommand, EngineContext, Event, Geometry,
    MeshHandle, PipelineDescriptor, PipelineHandle, Subsystem, Transform, WindowEvent,
    debug::DebugCameraController,
    math::{Quat, Vec3},
    shaders,
};

const RING_COUNT: u8 = 8;
const RING_RADIUS: f32 = 2.2;
const TRIANGLES: [(Vec3, f32); 3] = [
    (Vec3::new(-2.6, 0.9, -1.2), 0.7),
    (Vec3::new(2.6, 1.4, -2.0), 1.0),
    (Vec3::new(0.0, 2.2, -3.2), 1.4),
];

/// Démo multi-objets : 3 meshes partagés (cube, quad, triangle) et 2
/// pipelines (cullé pour les cubes fermés, double-sided pour les formes
/// plates) pour 13 draws par frame — un sol, un cube central en tumble, une
/// ronde de 8 cubes aux transforms tous différents, trois triangles
/// flottants. Toute géométrie est construite à l'origine et placée par le
/// Transform de son DrawCommand ; la soumission suit l'ordre de la scène, la
/// RenderQueue du moteur regroupe par pipeline. N'utilise que le vocabulaire
/// haut niveau — l'API d'un futur gamemode.
#[derive(Default)]
pub struct GeometryDemo {
    solid_pipeline: Option<PipelineHandle>,
    flat_pipeline: Option<PipelineHandle>,
    triangle: Option<MeshHandle>,
    quad: Option<MeshHandle>,
    cube: Option<MeshHandle>,
    elapsed: f32,
    camera: Camera,
    controller: DebugCameraController,
}

impl Subsystem for GeometryDemo {
    fn name(&self) -> &str {
        "geometry_demo"
    }

    fn init(&mut self, context: &mut EngineContext) -> ChaosResult<()> {
        let Some(renderer) = context.renderer_mut() else {
            return Ok(());
        };

        let solid = PipelineDescriptor::new("demo.geometry", shaders::builtin::VERTEX_COLOR)
            .with_vertex_layout(ColorVertex::layout())
            .with_cull_mode(CullMode::Back);
        self.solid_pipeline = Some(renderer.create_pipeline(&solid)?);

        let double_sided =
            PipelineDescriptor::new("demo.geometry.double_sided", shaders::builtin::VERTEX_COLOR)
                .with_vertex_layout(ColorVertex::layout());
        self.flat_pipeline = Some(renderer.create_pipeline(&double_sided)?);

        let triangle = Geometry::triangle(
            [0.0, 0.0, 0.0],
            1.0,
            [
                Color::rgb(0.95, 0.25, 0.85),
                Color::rgb(0.45, 0.15, 0.95),
                Color::rgb(0.20, 0.80, 0.95),
            ],
        );
        let quad = Geometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, Color::rgb(0.16, 0.12, 0.25));
        let cube = Geometry::cube(
            [0.0, 0.0, 0.0],
            1.0,
            [
                Color::rgb(0.95, 0.25, 0.85),
                Color::rgb(0.45, 0.15, 0.95),
                Color::rgb(0.20, 0.80, 0.95),
                Color::rgb(0.15, 0.35, 0.95),
                Color::rgb(0.75, 0.30, 0.95),
                Color::rgb(0.20, 0.95, 0.75),
            ],
        );

        self.triangle = Some(renderer.create_mesh("demo.triangle", &triangle)?);
        self.quad = Some(renderer.create_mesh("demo.quad", &quad)?);
        self.cube = Some(renderer.create_mesh("demo.cube", &cube)?);

        self.camera.transform.translation = Vec3::new(0.0, 1.2, 6.0);
        let (width, height) = renderer.surface_size();
        self.camera.set_viewport(width, height);
        Ok(())
    }

    fn on_event(&mut self, event: &Event, _context: &mut EngineContext) {
        self.controller.handle_event(event);
        if let Event::Window(WindowEvent::Resized { width, height }) = event {
            self.camera.set_viewport(*width, *height);
        }
    }

    fn update(&mut self, context: &mut EngineContext) {
        let (Some(solid), Some(flat), Some(triangle), Some(quad), Some(cube)) = (
            self.solid_pipeline,
            self.flat_pipeline,
            self.triangle,
            self.quad,
            self.cube,
        ) else {
            return;
        };
        let delta_seconds = context.time().delta_seconds();
        self.controller.update(&mut self.camera, delta_seconds);
        self.elapsed += delta_seconds;
        let t = self.elapsed;
        let Some(renderer) = context.renderer_mut() else {
            return;
        };
        renderer.set_view_projection(self.camera.view_projection());

        renderer.queue_draw(DrawCommand {
            pipeline: flat,
            mesh: quad,
            transform: Transform {
                translation: Vec3::new(0.0, -1.0, 0.0),
                rotation: Quat::from_rotation_x(-FRAC_PI_2),
                scale: Vec3::new(8.0, 8.0, 1.0),
            },
        });

        renderer.queue_draw(DrawCommand {
            pipeline: solid,
            mesh: cube,
            transform: Transform::from_rotation(
                Quat::from_rotation_y(0.9 * t) * Quat::from_rotation_x(0.6 * t),
            ),
        });

        for index in 0..RING_COUNT {
            let i = f32::from(index);
            let angle = i * FRAC_PI_4 + (0.15 + 0.05 * i) * t;
            renderer.queue_draw(DrawCommand {
                pipeline: solid,
                mesh: cube,
                transform: Transform {
                    translation: Vec3::new(angle.cos(), 0.0, angle.sin()) * RING_RADIUS,
                    rotation: Quat::from_rotation_y((0.4 + 0.2 * i) * t),
                    scale: Vec3::splat(0.3 + 0.06 * i),
                },
            });
        }

        for (position, size) in TRIANGLES {
            renderer.queue_draw(DrawCommand {
                pipeline: flat,
                mesh: triangle,
                transform: Transform::from_translation(position).with_scale(Vec3::splat(size)),
            });
        }
    }
}
