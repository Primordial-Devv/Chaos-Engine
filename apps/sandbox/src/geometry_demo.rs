use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};
use std::path::PathBuf;

use chaos_engine::{
    Camera, ChaosError, ChaosResult, Color, ColorVertex, Component, CullMode, DrawCommand,
    EngineContext, Entity, Event, Geometry, MaterialDescriptor, MaterialHandle, MeshHandle,
    PipelineDescriptor, SamplerDescriptor, SamplerFilter, Subsystem, System, TextureFormat,
    TexturedGeometry, TexturedVertex, Time, Transform, WindowEvent, World,
    assets::{AssetKind, AssetSource, ImportedAsset, texture_descriptor, textured_geometry},
    debug::DebugCameraController,
    math::{Quat, Vec3},
    shaders, stages,
};

const RING_COUNT: u8 = 8;
const RING_RADIUS: f32 = 2.2;

/// La rotation du cube central, en DONNÉES — le contenu définit ses
/// composants, le moteur fournit le mécanisme.
struct Spin {
    yaw_rate: f32,
    pitch_rate: f32,
}

impl Component for Spin {}

/// Le système du spin : lit le temps du monde, oriente tout porteur de
/// `Spin`. Il vit dans l'app, pas dans le moteur — l'API d'un futur
/// gamemode.
struct SpinSystem;

impl System for SpinSystem {
    fn name(&self) -> &str {
        "demo.spin"
    }

    fn run(&self, world: &mut World) -> ChaosResult<()> {
        let elapsed = world
            .resource::<Time>()
            .map(|time| time.elapsed.as_secs_f32())
            .unwrap_or_default();
        for (_, transform, spin) in world.query2_mut::<Transform, Spin>()? {
            transform.rotation = Quat::from_rotation_y(spin.yaw_rate * elapsed)
                * Quat::from_rotation_x(spin.pitch_rate * elapsed);
        }
        Ok(())
    }
}
const TRIANGLES: [(Vec3, f32); 3] = [
    (Vec3::new(-2.6, 0.9, -1.2), 0.7),
    (Vec3::new(2.6, 1.4, -2.0), 1.0),
    (Vec3::new(0.0, 2.2, -3.2), 1.4),
];

/// Démo multi-objets pilotée par les MATERIALS : chaque draw est le triplet
/// mesh + material + transform. **Le sol vient de fichiers via l'Asset
/// Pipeline** (texture `assets/textures/checker.ppm`, mesh
/// `assets/models/floor.glb` — déclarés, importés puis cousus vers le
/// renderer par `chaos_engine::assets`) ; le reste est procédural. 4 meshes
/// partagés, 4 pipelines, 4 materials — dont deux (`demo.floor` violet,
/// `demo.cube` ambre) partageant le MÊME damier : la couleur vient du
/// `base_color` du material. 13 draws par frame. **Le cube central est une
/// entité ECS** : son orientation vit dans un composant `Transform`, sa
/// vitesse dans un composant `Spin`, et c'est le système `demo.spin`
/// (enregistré dans `stages::UPDATE`) qui l'anime depuis la ressource
/// `Time` — l'update ne fait que lire le monde pour bâtir le DrawCommand.
/// Lancer depuis la racine du workspace (chemins d'assets relatifs).
/// N'utilise que le vocabulaire haut niveau — l'API d'un futur gamemode.
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
    cube_entity: Option<Entity>,
    elapsed: f32,
    camera: Camera,
    controller: DebugCameraController,
}

impl Subsystem for GeometryDemo {
    fn name(&self) -> &str {
        "geometry_demo"
    }

    fn init(&mut self, context: &mut EngineContext) -> ChaosResult<()> {
        if context.renderer().is_none() {
            return Ok(());
        }

        let assets = context.assets_mut();
        let checker_id = assets.declare(
            "textures/checker",
            AssetKind::Texture,
            AssetSource::File(PathBuf::from("assets/textures/checker.ppm")),
        )?;
        let checker_data = match assets.import(checker_id)? {
            ImportedAsset::Texture(data) => data.clone(),
            other => {
                return Err(ChaosError::Asset(format!(
                    "unexpected import kind for 'textures/checker': {other:?}"
                )));
            }
        };
        let floor_id = assets.declare(
            "models/floor",
            AssetKind::Mesh,
            AssetSource::File(PathBuf::from("assets/models/floor.glb")),
        )?;
        let floor_data = match assets.import(floor_id)? {
            ImportedAsset::Mesh(data) => data.clone(),
            other => {
                return Err(ChaosError::Asset(format!(
                    "unexpected import kind for 'models/floor': {other:?}"
                )));
            }
        };

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

        let checker = renderer.create_texture(&texture_descriptor(
            "demo.checker",
            &checker_data,
            TextureFormat::Rgba8UnormSrgb,
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
        let floor_quad = textured_geometry("models/floor", &floor_data)?;
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

        let cube_entity = context.world_mut().spawn()?;
        context
            .world_mut()
            .insert(cube_entity, Transform::IDENTITY)?;
        context.world_mut().insert(
            cube_entity,
            Spin {
                yaw_rate: 0.9,
                pitch_rate: 0.6,
            },
        )?;
        context
            .schedule_mut()
            .add_system(stages::UPDATE, SpinSystem)?;
        self.cube_entity = Some(cube_entity);
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
        let cube_transform = self
            .cube_entity
            .and_then(|entity| context.world().get::<Transform>(entity))
            .copied()
            .unwrap_or(Transform::IDENTITY);
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
            transform: cube_transform,
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
