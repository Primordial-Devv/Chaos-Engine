//! L'ÉCRAN DE SURVEILLANCE de la démo : la ronde vue du dessus, rendue
//! CHAQUE frame dans une cible hors écran par la passe déclarée
//! `demo.mirror` (ordre -10, caméra fixe), puis la couleur de la cible
//! affichée sur un quad flottant de la scène principale — rendre hors
//! écran, puis utiliser le résultat.

use chaos_engine::{
    Camera, ChaosResult, Color, DrawCommand, MaterialDescriptor, MaterialHandle, MaterialModel,
    MeshHandle, PassHandle, PassLoad, RenderDestination, RenderPassDescriptor,
    RenderTargetDescriptor, RenderTargetHandle, Renderer, SamplerAddressMode, SamplerDescriptor,
    TextureFormat, TexturedGeometry, Transform,
    math::{Quat, Vec3},
};

use super::stage::Stage;

/// Le rig du miroir : la cible 256×256, le quad-écran qui l'affiche, la
/// caméra fixe vue du dessus, et la passe déclarée (en DEUX temps — la
/// passe est déclarée en dernier dans l'init, l'ordre historique des
/// ressources).
pub(super) struct Mirror {
    target: RenderTargetHandle,
    screen_mesh: MeshHandle,
    screen_material: MaterialHandle,
    camera: Camera,
    pass: Option<PassHandle>,
}

impl Mirror {
    /// Crée la cible, le sampler ClampToEdge, le quad-écran et son
    /// material (la couleur de la cible comme n'importe quelle texture),
    /// et règle la caméra fixe — la passe sera déclarée par
    /// [`Mirror::declare_pass`].
    pub(super) fn build(renderer: &mut Renderer) -> ChaosResult<Self> {
        let target = renderer.create_render_target(&RenderTargetDescriptor::new(
            "demo.mirror",
            256,
            256,
            TextureFormat::Rgba8UnormSrgb,
        ))?;
        let screen_sampler = renderer.create_sampler(
            &SamplerDescriptor::new("demo.screen_sampler")
                .with_address_mode(SamplerAddressMode::ClampToEdge),
        )?;
        let screen_quad = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.6, 1.6, 1.0);
        let screen_mesh = renderer.create_textured_mesh("demo.screen", &screen_quad)?;
        let mirror_color = renderer.render_target_color(target)?;
        let screen_material = renderer.create_material(
            &MaterialDescriptor::new("demo.screen", MaterialModel::Unlit)
                .double_sided()
                .with_texture(mirror_color)
                .with_sampler(screen_sampler),
        )?;

        let mut camera = Camera::default();
        camera.transform.translation = Vec3::new(0.0, 7.0, 0.01);
        camera.transform.rotation = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        camera.set_viewport(256, 256);

        Ok(Self {
            target,
            screen_mesh,
            screen_material,
            camera,
            pass: None,
        })
    }

    /// Déclare la passe `demo.mirror` (ordre -10 — avant la principale) :
    /// la frame devient ORCHESTRÉE en deux passes.
    pub(super) fn declare_pass(&mut self, renderer: &mut Renderer) -> ChaosResult<()> {
        self.pass = Some(
            renderer.add_pass(
                &RenderPassDescriptor::new("demo.mirror", RenderDestination::Target(self.target))
                    .with_load(PassLoad::Clear(Color::rgb(0.05, 0.02, 0.10)))
                    .with_camera(self.camera.view_projection())
                    .with_camera_position(self.camera.transform.translation)
                    .with_order(-10),
            )?,
        );
        Ok(())
    }

    /// La vue-projection de la caméra FIXE du miroir — le frustum que
    /// le debug dessine (F) : on VOIT ce que l'écran de surveillance
    /// regarde.
    pub(super) fn view_projection(&self) -> chaos_engine::math::Mat4 {
        self.camera.view_projection()
    }

    /// Les draws du miroir pour cette frame : la ronde redessinée dans
    /// la passe déclarée avec le MÊME material que la scène (la
    /// permutation offscreen se résout seule), puis le quad-écran dans
    /// la scène principale.
    pub(super) fn frame(&self, renderer: &mut Renderer, stage: &Stage, ring: &[Transform]) {
        let Some(pass) = self.pass else {
            return;
        };
        for transform in ring {
            let command = DrawCommand {
                mesh: stage.ring_mesh(),
                material: stage.ring_material(),
                transform: *transform,
            };
            if let Err(mirror_error) = renderer.queue_draw_to(pass, command) {
                log::warn!("mirror pass draw rejected: {mirror_error}");
            }
        }
        renderer.queue_draw(DrawCommand {
            mesh: self.screen_mesh,
            material: self.screen_material,
            transform: Transform::from_translation(Vec3::new(-7.0, 1.6, 0.0)),
        });
    }
}
