//! L'ESSAIM de la démo : 1 200 mini-cubes éclairés — UN mesh + UN
//! material, soumis INDIVIDUELLEMENT chaque frame en double hélice
//! animée au-dessus du centre. Le renderer les fusionne SEUL en un
//! draw instancié par passe (principale ET ombre) : la preuve vivante
//! de l'instancing automatique — ~2 400 objets logiques de plus par
//! frame pour deux soumissions GPU.

use chaos_engine::{
    ChaosResult, Color, DrawCommand, LitGeometry, MaterialDescriptor, MaterialHandle,
    MaterialModel, MeshHandle, Renderer, Transform,
    math::{Quat, Vec3},
};

/// Le nombre de mini-cubes de l'essaim.
const SWARM_COUNT: u32 = 1200;

/// Le rig de l'essaim : un seul mesh, un seul material — la foule est
/// dans les transforms, recalculés chaque frame (le coût de soumission
/// est réel, et il est absorbé par le regroupement).
pub(super) struct Swarm {
    mesh: MeshHandle,
    material: MaterialHandle,
}

impl Swarm {
    /// Crée le mini-cube partagé et son material doré — un `Lit`
    /// ordinaire : il reçoit les ombres et en projette (l'essaim entier
    /// est UN caster instancié de plus dans la passe d'ombre).
    pub(super) fn build(renderer: &mut Renderer) -> ChaosResult<Self> {
        let cube = LitGeometry::cube([0.0, 0.0, 0.0], 1.0);
        let mesh = renderer.create_lit_mesh("demo.swarm", &cube)?;
        let material = renderer.create_material(
            &MaterialDescriptor::new("demo.swarm", MaterialModel::Lit)
                .with_base_color(Color::rgb(0.95, 0.8, 0.45)),
        )?;
        Ok(Self { mesh, material })
    }

    /// Les 1 200 draws de la frame : une double hélice qui tourne
    /// au-dessus du centre — chaque cube soumis par le triplet classique
    /// `DrawCommand`, comme n'importe quel objet. Aucune API de batch :
    /// le renderer décide seul.
    pub(super) fn frame(&self, renderer: &mut Renderer, t: f32) {
        for index in 0..SWARM_COUNT {
            let progress = index as f32 / SWARM_COUNT as f32;
            let strand = if index % 2 == 0 {
                0.0
            } else {
                std::f32::consts::PI
            };
            let angle = progress * std::f32::consts::TAU * 3.0 + strand + t * 0.35;
            let translation = Vec3::new(
                angle.cos() * 5.0,
                1.8 + progress * 4.2 + (t * 0.9 + progress * std::f32::consts::TAU).sin() * 0.2,
                angle.sin() * 5.0,
            );
            renderer.queue_draw(DrawCommand {
                mesh: self.mesh,
                material: self.material,
                transform: Transform {
                    translation,
                    rotation: Quat::from_rotation_y(angle),
                    scale: Vec3::splat(0.09),
                },
            });
        }
    }
}
