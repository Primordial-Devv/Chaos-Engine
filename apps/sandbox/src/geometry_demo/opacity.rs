//! L'OPACITÉ de la démo : le TRIO de verres transparents étagés en
//! profondeur (le panneau bleu pulsant + deux panneaux fixes — le
//! survol caméra prouve le tri arrière → avant), et la GRILLE masked
//! (alpha cutout) aux pastilles transparentes — les trous nets à
//! l'écran, l'ombre au sol en silhouette PLEINE : l'artefact V1 assumé
//! des casters non alpha-testés.

use chaos_engine::{
    ChaosResult, Color, DrawCommand, LitGeometry, MaterialDescriptor, MaterialHandle,
    MaterialModel, MaterialOpacity, MeshHandle, Renderer, TextureDescriptor, TextureFormat,
    TexturedGeometry, Transform, math::Vec3,
};

use super::content::grille_pixels;

/// Les deux panneaux de verre FIXES ajoutés au panneau pulsant : teinte
/// et profondeur — étagés en z autour du panneau pulsant (z = 0), le
/// survol caméra prouve le tri arrière → avant des transparents.
const GLASS_ROW: [(Color, f32); 2] = [
    (Color::rgba(1.0, 0.35, 0.3, 0.35), 1.2),
    (Color::rgba(0.35, 1.0, 0.4, 0.35), -1.2),
];

/// Le rig d'opacité : le mesh de panneau partagé, le verre pulsant, les
/// deux verres fixes et la grille masked.
pub(super) struct Opacity {
    glass_mesh: MeshHandle,
    glass_material: MaterialHandle,
    glass_row_materials: Vec<MaterialHandle>,
    grille_mesh: MeshHandle,
    grille_material: MaterialHandle,
}

impl Opacity {
    /// Crée le panneau de verre pulsant (LE material transparent
    /// historique de la démo — son alpha pulse via `set_material_color`,
    /// la mise à jour in-place : 16 octets écrits, zéro recréation), ses
    /// deux voisins fixes, puis la grille masked : une texture
    /// procédurale à TROUS (alpha 0 dans les pastilles) sur un quad
    /// éclairé double-sided — `fs_masked` élimine sous le cutoff, la
    /// profondeur s'écrit comme un opaque.
    pub(super) fn build(renderer: &mut Renderer) -> ChaosResult<Self> {
        let glass_quad = TexturedGeometry::quad([0.0, 0.0, 0.0], 2.4, 1.4, 1.0);
        let glass_mesh = renderer.create_textured_mesh("demo.glass", &glass_quad)?;
        let glass_material = renderer.create_material(
            &MaterialDescriptor::new("demo.glass", MaterialModel::Unlit)
                .double_sided()
                .with_opacity(MaterialOpacity::Transparent)
                .with_base_color(Color::rgba(0.35, 0.65, 1.0, 0.35)),
        )?;
        let mut glass_row_materials = Vec::new();
        for (index, (color, _)) in GLASS_ROW.iter().enumerate() {
            glass_row_materials.push(
                renderer.create_material(
                    &MaterialDescriptor::new(format!("demo.glass.{index}"), MaterialModel::Unlit)
                        .double_sided()
                        .with_opacity(MaterialOpacity::Transparent)
                        .with_base_color(*color),
                )?,
            );
        }

        let grille_texture = renderer.create_texture(&TextureDescriptor::sampled(
            "demo.grille",
            64,
            64,
            TextureFormat::Rgba8UnormSrgb,
            grille_pixels(64),
        ))?;
        let grille_quad = LitGeometry::quad([0.0, 0.0, 0.0], 2.0, 1.4, 1.0);
        let grille_mesh = renderer.create_lit_mesh("demo.grille", &grille_quad)?;
        let grille_material = renderer.create_material(
            &MaterialDescriptor::new("demo.grille", MaterialModel::Lit)
                .double_sided()
                .with_opacity(MaterialOpacity::Masked)
                .with_texture(grille_texture),
        )?;

        Ok(Self {
            glass_mesh,
            glass_material,
            glass_row_materials,
            grille_mesh,
            grille_material,
        })
    }

    /// Les draws d'opacité de la frame : le verre pulsant (l'alpha
    /// change chaque frame sans recréer une seule ressource) et ses deux
    /// voisins fixes — quel que soit l'angle de survol, les panneaux se
    /// mélangent dans le bon ordre (le tri arrière → avant suit la
    /// caméra) — puis la grille masked, immobile dans le coin : les
    /// pastilles transparentes laissent voir la scène au travers,
    /// l'ombre au sol reste la silhouette pleine (la limite V1,
    /// assumée).
    pub(super) fn frame(&self, renderer: &mut Renderer, t: f32) {
        // Le coin OPACITÉ, au bord droit : les trois verres étagés le
        // long du bord, la grille masked fermant le coin devant eux —
        // depuis l'apparition de la caméra, on regarde les verres À
        // TRAVERS les trous de la grille.
        let alpha = 0.2 + 0.25 * (t * 1.5).sin().abs();
        if let Err(glass_error) =
            renderer.set_material_color(self.glass_material, Color::rgba(0.35, 0.65, 1.0, alpha))
        {
            log::warn!("glass tint update failed: {glass_error}");
        }
        renderer.queue_draw(DrawCommand {
            mesh: self.glass_mesh,
            material: self.glass_material,
            transform: Transform::from_translation(Vec3::new(7.0, -0.3, 0.0)),
        });
        for (material, (_, z)) in self.glass_row_materials.iter().zip(GLASS_ROW.iter()) {
            renderer.queue_draw(DrawCommand {
                mesh: self.glass_mesh,
                material: *material,
                transform: Transform::from_translation(Vec3::new(7.0, -0.3, *z)),
            });
        }

        renderer.queue_draw(DrawCommand {
            mesh: self.grille_mesh,
            material: self.grille_material,
            transform: Transform::from_translation(Vec3::new(7.0, -0.3, 3.2)),
        });
    }
}
