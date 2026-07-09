use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

use chaos_engine::{
    Camera, ChaosResult, Color, ColorVertex, CullMode, DrawCommand, EngineContext, Event, Geometry,
    MaterialDescriptor, MaterialHandle, MeshHandle, PipelineDescriptor, SamplerDescriptor,
    SamplerFilter, Subsystem, TextureDescriptor, TextureFormat, TexturedGeometry, TexturedVertex,
    Transform, WindowEvent,
    debug::DebugCameraController,
    math::{Quat, Vec3},
    shaders, srgb8_bytes_of,
};

const RING_COUNT: u8 = 8;
const RING_RADIUS: f32 = 2.2;
const TRIANGLES: [(Vec3, f32); 3] = [
    (Vec3::new(-2.6, 0.9, -1.2), 0.7),
    (Vec3::new(2.6, 1.4, -2.0), 1.0),
    (Vec3::new(0.0, 2.2, -3.2), 1.4),
];

/// Démo multi-objets pilotée par les MATERIALS : chaque draw est le triplet
/// mesh + material + transform. 4 meshes partagés (triangle, sol texturé,
/// cube coloré, cube texturé), 4 pipelines, 4 materials — dont deux
/// (`demo.floor` violet, `demo.cube` ambre) partageant le MÊME damier
/// blanc/gris : la couleur vient du paramètre `base_color` du material, la
/// texture reste neutre. 13 draws par frame : sol damier, cube central
/// texturé en tumble (UV par face), ronde de 8 cubes colorés, trois
/// triangles flottants. Toute géométrie est construite à l'origine et
/// placée par le Transform de son DrawCommand ; la RenderQueue regroupe par
/// material. N'utilise que le vocabulaire haut niveau — l'API d'un futur
/// gamemode.
#[derive(Default)]
pub struct GeometryDemo {
    solid_material: Option<MaterialHandle>,
    flat_material: Option<MaterialHandle>,
    floor_material: Option<MaterialHandle>,
    cube_material: Option<MaterialHandle>,
    triangle: Option<MeshHandle>,
    floor: Option<MeshHandle>,
    colored_cube: Option<MeshHandle>,
    textured_cube: Option<MeshHandle>,
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
        let solid_pipeline = renderer.create_pipeline(&solid)?;

        let double_sided =
            PipelineDescriptor::new("demo.geometry.double_sided", shaders::builtin::VERTEX_COLOR)
                .with_vertex_layout(ColorVertex::layout());
        let flat_pipeline = renderer.create_pipeline(&double_sided)?;

        let floor = PipelineDescriptor::new("demo.floor", shaders::builtin::TEXTURED)
            .with_vertex_layout(TexturedVertex::layout())
            .with_material();
        let floor_pipeline = renderer.create_pipeline(&floor)?;

        let textured_solid = PipelineDescriptor::new("demo.textured", shaders::builtin::TEXTURED)
            .with_vertex_layout(TexturedVertex::layout())
            .with_cull_mode(CullMode::Back)
            .with_material();
        let textured_pipeline = renderer.create_pipeline(&textured_solid)?;

        let checker = renderer.create_texture(&TextureDescriptor::sampled(
            "demo.checker",
            2,
            2,
            TextureFormat::Rgba8UnormSrgb,
            srgb8_bytes_of(&[
                Color::WHITE,
                Color::rgb(0.45, 0.45, 0.45),
                Color::rgb(0.45, 0.45, 0.45),
                Color::WHITE,
            ]),
        ))?;
        let checker_sampler = renderer.create_sampler(
            &SamplerDescriptor::new("demo.checker_sampler").with_filter(SamplerFilter::Nearest),
        )?;

        self.solid_material =
            Some(renderer.create_material(&MaterialDescriptor::new("demo.solid", solid_pipeline))?);
        self.flat_material =
            Some(renderer.create_material(&MaterialDescriptor::new("demo.flat", flat_pipeline))?);
        self.floor_material = Some(
            renderer.create_material(
                &MaterialDescriptor::new("demo.floor", floor_pipeline)
                    .with_base_color(Color::rgb(0.62, 0.44, 0.92))
                    .with_texture(checker)
                    .with_sampler(checker_sampler),
            )?,
        );
        self.cube_material = Some(
            renderer.create_material(
                &MaterialDescriptor::new("demo.cube", textured_pipeline)
                    .with_base_color(Color::rgb(0.95, 0.55, 0.20))
                    .with_texture(checker)
                    .with_sampler(checker_sampler),
            )?,
        );

        let triangle = Geometry::triangle(
            [0.0, 0.0, 0.0],
            1.0,
            [
                Color::rgb(0.95, 0.25, 0.85),
                Color::rgb(0.45, 0.15, 0.95),
                Color::rgb(0.20, 0.80, 0.95),
            ],
        );
        let floor_quad = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 4.0);
        let textured_cube = TexturedGeometry::cube([0.0, 0.0, 0.0], 1.0);
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
        self.floor = Some(renderer.create_textured_mesh("demo.floor", &floor_quad)?);
        self.colored_cube = Some(renderer.create_mesh("demo.cube", &cube)?);
        self.textured_cube =
            Some(renderer.create_textured_mesh("demo.textured_cube", &textured_cube)?);

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
        let (
            Some(solid),
            Some(flat),
            Some(floor_material),
            Some(cube_material),
            Some(triangle),
            Some(floor),
            Some(colored_cube),
            Some(textured_cube),
        ) = (
            self.solid_material,
            self.flat_material,
            self.floor_material,
            self.cube_material,
            self.triangle,
            self.floor,
            self.colored_cube,
            self.textured_cube,
        )
        else {
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
            mesh: floor,
            material: floor_material,
            transform: Transform {
                translation: Vec3::new(0.0, -1.0, 0.0),
                rotation: Quat::from_rotation_x(-FRAC_PI_2),
                scale: Vec3::new(8.0, 8.0, 1.0),
            },
        });

        renderer.queue_draw(DrawCommand {
            mesh: textured_cube,
            material: cube_material,
            transform: Transform::from_rotation(
                Quat::from_rotation_y(0.9 * t) * Quat::from_rotation_x(0.6 * t),
            ),
        });

        for index in 0..RING_COUNT {
            let i = f32::from(index);
            let angle = i * FRAC_PI_4 + (0.15 + 0.05 * i) * t;
            renderer.queue_draw(DrawCommand {
                mesh: colored_cube,
                material: solid,
                transform: Transform {
                    translation: Vec3::new(angle.cos(), 0.0, angle.sin()) * RING_RADIUS,
                    rotation: Quat::from_rotation_y((0.4 + 0.2 * i) * t),
                    scale: Vec3::splat(0.3 + 0.06 * i),
                },
            });
        }

        for (position, size) in TRIANGLES {
            renderer.queue_draw(DrawCommand {
                mesh: triangle,
                material: flat,
                transform: Transform::from_translation(position).with_scale(Vec3::splat(size)),
            });
        }
    }
}
