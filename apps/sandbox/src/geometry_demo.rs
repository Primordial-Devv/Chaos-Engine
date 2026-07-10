use std::collections::HashMap;
use std::f32::consts::FRAC_PI_4;
use std::path::PathBuf;

use chaos_engine::{
    AssetId, Camera, ChaosError, ChaosResult, Color, ColorVertex, Component, CullMode, DrawCommand,
    EngineContext, Event, Geometry, GlobalTransform, MaterialDescriptor, MaterialHandle,
    MeshHandle, MeshRef, PipelineDescriptor, SamplerDescriptor, SamplerFilter, Scene, SceneId,
    Subsystem, System, TextureFormat, TexturedGeometry, TexturedVertex, Time, Transform,
    WindowEvent, World,
    assets::{AssetKind, AssetSource, ImportedAsset, texture_descriptor, textured_geometry},
    debug::DebugCameraController,
    hierarchy,
    math::{Quat, Vec3},
    scenes, shaders, stages,
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
/// `base_color` du material. 15 draws par frame. **Le sol, le cube central
/// et ses deux satellites sont du CONTENU : la scène `scenes/demo` est
/// chargée depuis le fichier committé `assets/scenes/demo.cscn` via
/// l'Asset Pipeline — aucune entité n'est construite dans ce code.** Les
/// entités portent des `MeshRef` (identités d'assets stables — le mesh
/// procédural du cube a la sienne via `AssetSource::Procedural`) résolus
/// au chargement puis par la table AssetId → handles du renderer. Le cube
/// tourne via son composant `Spin` (ré-attaché après chargement : le
/// comportement n'est pas des données de scène) animé par `demo.spin`
/// (`stages::UPDATE`), les satellites suivent par la seule propagation
/// (`stages::POST_UPDATE`), et l'update dessine les membres depuis leurs
/// `GlobalTransform`.
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
    scene_id: Option<SceneId>,
    mesh_table: HashMap<AssetId, (MeshHandle, MaterialHandle)>,
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
        let floor_mesh = renderer.create_textured_mesh("demo.floor", &floor_quad)?;
        self.floor = Some(floor_mesh);
        self.colored_cube = Some(renderer.create_mesh("demo.cube", &cube)?);
        let textured_cube_mesh =
            renderer.create_textured_mesh("demo.textured_cube", &textured_cube)?;
        self.textured_cube = Some(textured_cube_mesh);

        self.camera.transform.translation = Vec3::new(0.0, 1.2, 6.0);
        let (width, height) = renderer.surface_size();
        self.camera.set_viewport(width, height);

        context
            .schedule_mut()
            .add_system(stages::UPDATE, SpinSystem)?;

        // Le mesh procédural du cube reçoit une identité stable : l'Asset
        // Pipeline est l'espace de noms de résolution, même pour le
        // contenu généré. La table résout AssetId → handles du renderer.
        let cube_asset =
            context
                .assets_mut()
                .declare("demo/cube", AssetKind::Mesh, AssetSource::Procedural)?;
        let floor_material_handle = self.floor_material.ok_or_else(|| {
            ChaosError::Scene(String::from("floor material missing during scene setup"))
        })?;
        let cube_material_handle = self.cube_material.ok_or_else(|| {
            ChaosError::Scene(String::from("cube material missing during scene setup"))
        })?;
        self.mesh_table
            .insert(floor_id, (floor_mesh, floor_material_handle));
        self.mesh_table
            .insert(cube_asset, (textured_cube_mesh, cube_material_handle));

        // La scène de démo est du CONTENU : chargée depuis le fichier
        // committé `assets/scenes/demo.cscn` via l'Asset Pipeline —
        // aucune entité n'est construite dans ce code. Les références
        // (`models/floor`, `demo/cube`) sont résolues au chargement.
        let scene_asset = context.assets_mut().declare(
            "scenes/demo",
            AssetKind::Scene,
            AssetSource::File(PathBuf::from("assets/scenes/demo.cscn")),
        )?;
        let loaded = scenes::load_scene(context.assets_mut(), scene_asset)?;

        let (world, scene_manager) = context.world_and_scenes();
        let scene_id = scene_manager.create(&loaded.name)?;
        scene_manager.load(world, scene_id, |scene, world| loaded.apply(scene, world))?;
        scene_manager.activate(scene_id)?;
        // Le comportement n'est pas des données de scène (le futur
        // scripting) : le contenu ré-attache son Spin au parent rechargé.
        let Some(scene) = scene_manager.scene(scene_id) else {
            return Err(ChaosError::Scene(String::from("demo scene not registered")));
        };
        let spinner = scene
            .members(world)
            .find(|entity| hierarchy::children_of(world, *entity).next().is_some());
        if let Some(spinner) = spinner {
            world.insert(
                spinner,
                Spin {
                    yaw_rate: 0.9,
                    pitch_rate: 0.6,
                },
            )?;
        }
        self.scene_id = Some(scene_id);
        Ok(())
    }

    fn on_event(&mut self, event: &Event, _context: &mut EngineContext) {
        self.controller.handle_event(event);
        if let Event::Window(WindowEvent::Resized { width, height }) = event {
            self.camera.set_viewport(*width, *height);
        }
    }

    fn update(&mut self, context: &mut EngineContext) {
        let (Some(solid), Some(flat), Some(triangle), Some(colored_cube)) = (
            self.solid_material,
            self.flat_material,
            self.triangle,
            self.colored_cube,
        ) else {
            return;
        };
        let delta_seconds = context.time().delta_seconds();
        self.controller.update(&mut self.camera, delta_seconds);
        self.elapsed += delta_seconds;
        let t = self.elapsed;
        // La scène active se dessine par ses données : membres → MeshRef →
        // table de résolution → DrawCommand (GlobalTransform décomposé).
        let scene_draws: Vec<(Transform, MeshHandle, MaterialHandle)> = self
            .scene_id
            .and_then(|id| context.scenes().scene(id))
            .map(|scene: &Scene| {
                let world = context.world();
                scene
                    .members(world)
                    .filter_map(|entity| {
                        let mesh_ref = world.get::<MeshRef>(entity)?;
                        let (mesh, material) = self.mesh_table.get(&mesh_ref.mesh()).copied()?;
                        let global = world.get::<GlobalTransform>(entity)?;
                        let (scale, rotation, translation) =
                            global.matrix().to_scale_rotation_translation();
                        Some((
                            Transform {
                                translation,
                                rotation,
                                scale,
                            },
                            mesh,
                            material,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let Some(renderer) = context.renderer_mut() else {
            return;
        };
        renderer.set_view_projection(self.camera.view_projection());

        for (transform, mesh, material) in &scene_draws {
            renderer.queue_draw(DrawCommand {
                mesh: *mesh,
                material: *material,
                transform: *transform,
            });
        }

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
