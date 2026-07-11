//! La SCÈNE DE FOND de la démo : le sol et le cube central venus de
//! FICHIERS via l'Asset Pipeline (texture `checker.ppm`, mesh
//! `floor.glb`, scène `demo.cscn`), la ronde de cubes colorés et les
//! triangles flottants procéduraux. C'est la partie « contenu chargé +
//! primitives » — les showcases (miroir, PBR, opacité…) vivent dans
//! leurs propres rigs.

use std::collections::HashMap;
use std::f32::consts::FRAC_PI_4;
use std::path::PathBuf;

use chaos_engine::{
    AssetId, ChaosError, ChaosResult, Color, DrawCommand, EngineContext, Geometry, GlobalTransform,
    LitGeometry, MaterialDescriptor, MaterialHandle, MaterialModel, MeshHandle, MeshRef, Renderer,
    SamplerDescriptor, SamplerFilter, Scene, SceneId, TextureFormat, TextureMips, Transform,
    assets::{AssetKind, AssetSource, ImportedAsset, lit_geometry, texture_descriptor},
    hierarchy,
    math::{Quat, Vec3},
    scenes, stages,
};

use super::spin::{Spin, SpinSystem};

/// La ronde : huit cubes colorés partageant le MÊME mesh, orbites et
/// spins tous différents — ils se croisent en permanence, l'occlusion
/// doit rester correcte à chaque croisement.
const RING_COUNT: u8 = 8;
const RING_RADIUS: f32 = 2.2;

/// Les trois triangles dégradés flottants : position et échelle — un
/// TOTEM vertical dans le coin arrière-gauche, le plus grand en haut.
const TRIANGLES: [(Vec3, f32); 3] = [
    (Vec3::new(-7.0, 1.0, -7.0), 0.7),
    (Vec3::new(-7.0, 1.8, -7.0), 1.0),
    (Vec3::new(-7.0, 2.6, -7.0), 1.4),
];

/// Le rig de la scène de fond : les materials et meshes du contenu
/// (sol, cube central, ronde, triangles), la table AssetId → handles et
/// la scène chargée depuis `demo.cscn`.
pub(super) struct Stage {
    solid: MaterialHandle,
    flat: MaterialHandle,
    floor_material: MaterialHandle,
    cube_material: MaterialHandle,
    triangle: MeshHandle,
    floor_mesh: MeshHandle,
    colored_cube: MeshHandle,
    textured_cube: MeshHandle,
    floor_id: AssetId,
    mesh_table: HashMap<AssetId, (MeshHandle, MaterialHandle)>,
    scene_id: Option<SceneId>,
}

impl Stage {
    /// Déclare et importe les fichiers de la démo (damier PPM, sol GLB)
    /// puis matérialise le contenu chez le renderer : le damier mippé et
    /// ses deux samplers, les quatre materials du fond (solid, flat, sol
    /// violet, cube ambre — les deux derniers PARTAGENT le damier), et
    /// les quatre meshes (triangle, sol, cube coloré, cube texturé).
    /// `None` si le renderer est absent (le contrat historique du
    /// subsystem : silencieux, jamais une erreur).
    pub(super) fn build(context: &mut EngineContext) -> ChaosResult<Option<Self>> {
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
            return Ok(None);
        };

        let checker = renderer.create_texture(
            &texture_descriptor("demo.checker", &checker_data, TextureFormat::Rgba8UnormSrgb)
                .with_mips(TextureMips::Generate),
        )?;
        let checker_sampler = renderer.create_sampler(
            &SamplerDescriptor::new("demo.checker_sampler").with_filter(SamplerFilter::Nearest),
        )?;
        let floor_sampler = renderer.create_sampler(
            &SamplerDescriptor::new("demo.floor_sampler")
                .with_mip_filter(SamplerFilter::Linear)
                .with_anisotropy(8),
        )?;

        let solid = renderer.create_material(&MaterialDescriptor::new(
            "demo.solid",
            MaterialModel::VertexColor,
        ))?;
        let flat = renderer.create_material(
            &MaterialDescriptor::new("demo.flat", MaterialModel::VertexColor).double_sided(),
        )?;
        let floor_material = renderer.create_material(
            &MaterialDescriptor::new("demo.floor", MaterialModel::Lit)
                .double_sided()
                .with_base_color(Color::rgb(0.62, 0.44, 0.92))
                .with_texture(checker)
                .with_sampler(floor_sampler),
        )?;
        let cube_material = renderer.create_material(
            &MaterialDescriptor::new("demo.cube", MaterialModel::Lit)
                .with_base_color(Color::rgb(0.95, 0.55, 0.20))
                .with_texture(checker)
                .with_sampler(checker_sampler),
        )?;

        let triangle = Geometry::triangle(
            [0.0, 0.0, 0.0],
            1.0,
            [
                Color::rgb(0.95, 0.25, 0.85),
                Color::rgb(0.45, 0.15, 0.95),
                Color::rgb(0.20, 0.80, 0.95),
            ],
        );
        let floor_quad = lit_geometry("models/floor", &floor_data)?;
        let lit_cube = LitGeometry::cube([0.0, 0.0, 0.0], 1.0);
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

        Ok(Some(Self {
            solid,
            flat,
            floor_material,
            cube_material,
            triangle: renderer.create_mesh("demo.triangle", &triangle)?,
            floor_mesh: renderer.create_lit_mesh("demo.floor", &floor_quad)?,
            colored_cube: renderer.create_mesh("demo.cube", &cube)?,
            textured_cube: renderer.create_lit_mesh("demo.textured_cube", &lit_cube)?,
            floor_id,
            mesh_table: HashMap::new(),
            scene_id: None,
        }))
    }

    /// Charge la scène de démo — du CONTENU : le système `demo.spin` est
    /// enregistré, le mesh procédural du cube reçoit une identité stable
    /// (l'Asset Pipeline est l'espace de noms de résolution, même pour
    /// le contenu généré), la table résout AssetId → handles, puis la
    /// scène est chargée depuis le fichier committé
    /// `assets/scenes/demo.cscn` et activée — aucune entité n'est
    /// construite dans ce code. Le comportement n'est pas des données de
    /// scène (le futur scripting) : le contenu ré-attache son `Spin` au
    /// parent rechargé.
    pub(super) fn populate_scene(&mut self, context: &mut EngineContext) -> ChaosResult<()> {
        context
            .schedule_mut()
            .add_system(stages::UPDATE, SpinSystem)?;

        let cube_asset =
            context
                .assets_mut()
                .declare("demo/cube", AssetKind::Mesh, AssetSource::Procedural)?;
        self.mesh_table
            .insert(self.floor_id, (self.floor_mesh, self.floor_material));
        self.mesh_table
            .insert(cube_asset, (self.textured_cube, self.cube_material));

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

    /// La scène active se dessine par ses données : membres → `MeshRef`
    /// → table de résolution → DrawCommand (`GlobalTransform` décomposé).
    /// Collecté AVANT l'emprunt du renderer (le monde se lit ici).
    pub(super) fn collect_scene_draws(
        &self,
        context: &EngineContext,
    ) -> Vec<(Transform, MeshHandle, MaterialHandle)> {
        self.scene_id
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
            .unwrap_or_default()
    }

    /// Les draws du fond pour cette frame : les membres de la scène,
    /// puis la ronde (dont les transforms sont RENDUS à l'appelant — le
    /// miroir redessine les mêmes cubes), puis les triangles flottants.
    pub(super) fn frame(
        &self,
        renderer: &mut Renderer,
        scene_draws: &[(Transform, MeshHandle, MaterialHandle)],
        t: f32,
    ) -> Vec<Transform> {
        for (transform, mesh, material) in scene_draws {
            renderer.queue_draw(DrawCommand {
                mesh: *mesh,
                material: *material,
                transform: *transform,
            });
        }

        let mut ring_transforms = Vec::with_capacity(usize::from(RING_COUNT));
        for index in 0..RING_COUNT {
            let i = f32::from(index);
            let angle = i * FRAC_PI_4 + (0.15 + 0.05 * i) * t;
            let transform = Transform {
                translation: Vec3::new(angle.cos(), 0.0, angle.sin()) * RING_RADIUS,
                rotation: Quat::from_rotation_y((0.4 + 0.2 * i) * t),
                scale: Vec3::splat(0.3 + 0.06 * i),
            };
            ring_transforms.push(transform);
            renderer.queue_draw(DrawCommand {
                mesh: self.colored_cube,
                material: self.solid,
                transform,
            });
        }

        for (position, size) in TRIANGLES {
            renderer.queue_draw(DrawCommand {
                mesh: self.triangle,
                material: self.flat,
                transform: Transform::from_translation(position).with_scale(Vec3::splat(size)),
            });
        }

        ring_transforms
    }

    /// Le mesh partagé de la ronde — le miroir le redessine.
    pub(super) fn ring_mesh(&self) -> MeshHandle {
        self.colored_cube
    }

    /// Le material de la ronde — le MÊME sert la scène et le miroir (la
    /// permutation offscreen se résout seule).
    pub(super) fn ring_material(&self) -> MaterialHandle {
        self.solid
    }
}
