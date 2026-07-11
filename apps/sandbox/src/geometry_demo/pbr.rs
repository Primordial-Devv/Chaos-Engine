//! Le SHOWCASE PBR de la démo : la grille metallic × roughness (des
//! sphères gris clair — le gradient des highlights), le cube
//! normal-mappé (la map est PROCÉDURALE : des bosses sinusoïdales,
//! vecteurs normalisés, format LINÉAIRE sans mips) et la sphère
//! émissive PULSANTE (`set_material_emissive` in-place chaque frame —
//! le chemin contrôlé des paramètres PBR).

use chaos_engine::{
    ChaosResult, Color, DrawCommand, LitGeometry, MaterialDescriptor, MaterialHandle,
    MaterialModel, MeshHandle, Renderer, TextureDescriptor, TextureFormat, Transform,
    math::{Quat, Vec3},
};

use super::content::bumpy_normal_map;

/// La grille PBR de la démo : `PBR_GRID` × `PBR_GRID` sphères, metallic
/// croissant vers la droite, rugosité croissante vers le bas.
const PBR_GRID: usize = 4;

/// Le rig PBR : le mesh de sphère partagé, les 16 materials de la
/// grille, le cube normal-mappé et la sphère émissive.
pub(super) struct PbrShowcase {
    sphere_mesh: MeshHandle,
    grid_materials: Vec<MaterialHandle>,
    bumpy_mesh: MeshHandle,
    bumpy_material: MaterialHandle,
    glowing_material: MaterialHandle,
}

impl PbrShowcase {
    /// Crée la sphère UV partagée, les 16 permutations metallic ×
    /// roughness (UNE seule permutation de pipeline pour toutes), la
    /// normal map procédurale et son cube, et la sphère émissive — qui
    /// ne PROJETTE pas d'ombre (`without_shadow_cast`) : la preuve par
    /// l'absence, ses voisines de la grille projettent, elle non (et une
    /// boule de lumière qui assombrit le sol serait incongrue).
    pub(super) fn build(renderer: &mut Renderer) -> ChaosResult<Self> {
        let sphere = LitGeometry::sphere([0.0, 0.0, 0.0], 0.32, 24, 16);
        let sphere_mesh = renderer.create_lit_mesh("demo.sphere", &sphere)?;
        let mut grid_materials = Vec::new();
        for row in 0..PBR_GRID {
            for column in 0..PBR_GRID {
                let metallic = column as f32 / (PBR_GRID - 1) as f32;
                let roughness = 0.1 + 0.9 * (row as f32 / (PBR_GRID - 1) as f32);
                grid_materials.push(
                    renderer.create_material(
                        &MaterialDescriptor::new(
                            format!("demo.pbr.{row}{column}"),
                            MaterialModel::Pbr,
                        )
                        .with_base_color(Color::rgb(0.85, 0.85, 0.85))
                        .with_metallic(metallic)
                        .with_roughness(roughness),
                    )?,
                );
            }
        }
        let normal_map = renderer.create_texture(&TextureDescriptor::sampled(
            "demo.bumps",
            64,
            64,
            TextureFormat::Rgba8Unorm,
            bumpy_normal_map(64),
        ))?;
        let bumpy_cube = LitGeometry::cube([0.0, 0.0, 0.0], 0.9);
        let bumpy_mesh = renderer.create_lit_mesh("demo.bumpy", &bumpy_cube)?;
        let bumpy_material = renderer.create_material(
            &MaterialDescriptor::new("demo.bumpy", MaterialModel::Pbr)
                .with_base_color(Color::rgb(0.75, 0.55, 0.35))
                .with_roughness(0.5)
                .with_normal_map(normal_map),
        )?;
        let glowing_material = renderer.create_material(
            &MaterialDescriptor::new("demo.glowing", MaterialModel::Pbr)
                .with_base_color(Color::rgb(0.1, 0.1, 0.12))
                .with_roughness(0.6)
                .with_emissive(Color::rgb(2.0, 0.6, 0.1))
                .without_shadow_cast(),
        )?;
        Ok(Self {
            sphere_mesh,
            grid_materials,
            bumpy_mesh,
            bumpy_material,
            glowing_material,
        })
    }

    /// Les draws PBR de la frame : la grille flottant derrière la scène,
    /// la sphère émissive dont l'intensité PULSE (mise à jour in-place),
    /// et le cube normal-mappé en rotation lente qui accroche les
    /// ponctuelles orbitantes sur ses bosses.
    pub(super) fn frame(&self, renderer: &mut Renderer, t: f32) {
        for (index, material) in self.grid_materials.iter().enumerate() {
            let row = index / PBR_GRID;
            let column = index % PBR_GRID;
            let position = Vec3::new(-1.35 + column as f32 * 0.9, 3.4 - row as f32 * 0.9, -7.0);
            renderer.queue_draw(DrawCommand {
                mesh: self.sphere_mesh,
                material: *material,
                transform: Transform::from_translation(position),
            });
        }
        let glow = 1.1 + (t * 2.2).sin() * 0.9;
        if let Err(glow_error) = renderer.set_material_emissive(
            self.glowing_material,
            Color::rgb(2.0 * glow, 0.6 * glow, 0.1 * glow),
        ) {
            log::warn!("emissive update failed: {glow_error}");
        }
        // Les deux pièces uniques FLANQUENT la grille sur le mur du
        // fond : l'émissive à droite, le normal-mappé à gauche — le
        // showcase PBR est UN pavillon.
        renderer.queue_draw(DrawCommand {
            mesh: self.sphere_mesh,
            material: self.glowing_material,
            transform: Transform::from_translation(Vec3::new(3.0, 0.4, -7.0)),
        });
        renderer.queue_draw(DrawCommand {
            mesh: self.bumpy_mesh,
            material: self.bumpy_material,
            transform: Transform {
                translation: Vec3::new(-3.0, 0.4, -7.0),
                rotation: Quat::from_rotation_y(0.3 * t),
                scale: Vec3::ONE,
            },
        });
    }
}
