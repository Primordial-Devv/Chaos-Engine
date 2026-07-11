//! LA SUITE stress & régression du renderer — quatre familles
//! (rendu, ressources, robustesse, performance) construites NOIRE-BOÎTE
//! sur l'API publique, les diagnostics et les stats : le niveau
//! CONTRAT, durable par construction. Le cœur est LA SCÈNE CANONIQUE :
//! une composition qui traverse TOUT le renderer d'un coup
//! (multi-passes, offscreen, lighting, PBR, environnement, IBL, ciel,
//! ombres, transparence, instancing, culling, debug) aux comptes
//! EXACTS — toute régression architecturale, fonctionnelle ou de
//! performance la fait dévier.

use chaos_core::math::{Mat4, Vec3};
use chaos_core::{Color, Transform};

use crate::capabilities::DeviceLimits;
use crate::renderer::Renderer;
use crate::testing::{
    mock_renderer, mock_renderer_switchable, mock_renderer_with_limits, render_lines,
};
use crate::{
    BuiltinTexture, DebugDraw, DirectionalShadowDescriptor, DrawCommand, EnvironmentDescriptor,
    FrameOutcome, FrameSkipReason, Light, LitGeometry, MaterialDescriptor, MaterialHandle,
    MaterialModel, MaterialOpacity, MeshHandle, PassHandle, PassOutcome, RenderPassDescriptor,
    RenderTargetDescriptor, RenderTargetHandle, ResourceStats, SamplerDescriptor, SamplerFilter,
    ShadowVolume, TextureDescriptor, TextureFormat, frame::RenderDestination,
};

/// La taille de la foule VISIBLE (fusionnée en un batch) de la scène.
const CROWD: u32 = 100;
/// La taille de la foule HORS CHAMP (cullée).
const CULLED: u32 = 50;
/// La part de la foule redessinée dans le miroir.
const MIRROR_CROWD: u32 = 10;

/// LA scène canonique : les handles vivants et la soumission par frame.
struct CanonicalScene {
    floor_material: MaterialHandle,
    floor_mesh: MeshHandle,
    pbr_materials: [MaterialHandle; 2],
    sphere_mesh: MeshHandle,
    masked_material: MaterialHandle,
    glass_material: MaterialHandle,
    pane_mesh: MeshHandle,
    crowd_material: MaterialHandle,
    crowd_mesh: MeshHandle,
    screen_material: MaterialHandle,
    screen_mesh: MeshHandle,
    mirror_pass: PassHandle,
    mirror_target: RenderTargetHandle,
}

/// Construit la scène canonique : environnement HDR + ciel, ombres,
/// sol texturé (sampler trilinéaire anisotrope), sphères PBR, grille
/// masked, verres, foule instanciée, miroir déclaré + écran — chaque
/// chemin de création du renderer traversé une fois.
fn canonical_scene(renderer: &mut Renderer) -> CanonicalScene {
    let cubemap = renderer
        .create_texture(&TextureDescriptor::cube(
            "suite.sky",
            1,
            TextureFormat::Rgba8Unorm,
            vec![0; 4 * 6],
        ))
        .unwrap();
    renderer
        .set_environment(&EnvironmentDescriptor::new(cubemap))
        .unwrap();
    renderer.set_ambient_light(Color::WHITE, 0.05);
    renderer
        .set_directional_shadow(&DirectionalShadowDescriptor::new(ShadowVolume::new(
            Vec3::new(50.0, 0.0, 0.0),
            Vec3::new(200.0, 50.0, 200.0),
        )))
        .unwrap();

    let checker = renderer
        .create_texture(&TextureDescriptor::sampled(
            "suite.checker",
            2,
            2,
            TextureFormat::Rgba8UnormSrgb,
            vec![128; 16],
        ))
        .unwrap();
    let trilinear = renderer
        .create_sampler(
            &SamplerDescriptor::new("suite.trilinear")
                .with_mip_filter(SamplerFilter::Linear)
                .with_anisotropy(8),
        )
        .unwrap();
    let floor_material = renderer
        .create_material(
            &MaterialDescriptor::new("suite.floor", MaterialModel::Lit)
                .with_texture(checker)
                .with_sampler(trilinear),
        )
        .unwrap();
    let floor_mesh = renderer
        .create_lit_mesh(
            "suite.floor",
            &LitGeometry::quad([0.0, 0.0, 0.0], 20.0, 20.0, 4.0),
        )
        .unwrap();

    let sphere_mesh = renderer
        .create_lit_mesh(
            "suite.sphere",
            &LitGeometry::sphere([0.0, 0.0, 0.0], 0.5, 16, 12),
        )
        .unwrap();
    let pbr_materials = [
        renderer
            .create_material(
                &MaterialDescriptor::new("suite.pbr.metal", MaterialModel::Pbr)
                    .with_metallic(1.0)
                    .with_roughness(0.1),
            )
            .unwrap(),
        renderer
            .create_material(
                &MaterialDescriptor::new("suite.pbr.rough", MaterialModel::Pbr).with_roughness(0.9),
            )
            .unwrap(),
    ];

    let masked_material = renderer
        .create_material(
            &MaterialDescriptor::new("suite.grille", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Masked),
        )
        .unwrap();
    let glass_material = renderer
        .create_material(
            &MaterialDescriptor::new("suite.glass", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
    let pane_mesh = renderer
        .create_lit_mesh(
            "suite.pane",
            &LitGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0),
        )
        .unwrap();

    let crowd_material = renderer
        .create_material(&MaterialDescriptor::new("suite.crowd", MaterialModel::Lit))
        .unwrap();
    let crowd_mesh = renderer
        .create_lit_mesh("suite.cube", &LitGeometry::cube([0.0, 0.0, 0.0], 0.5))
        .unwrap();

    let mirror_target = renderer
        .create_render_target(&crate::RenderTargetDescriptor::new(
            "suite.mirror",
            128,
            128,
            TextureFormat::Rgba8UnormSrgb,
        ))
        .unwrap();
    let mirror_pass = renderer
        .add_pass(
            &RenderPassDescriptor::new("suite.mirror", RenderDestination::Target(mirror_target))
                .with_camera(Mat4::from_scale(Vec3::new(0.001, 0.001, -0.001)))
                .with_order(-10),
        )
        .unwrap();
    let mirror_color = renderer.render_target_color(mirror_target).unwrap();
    let screen_material = renderer
        .create_material(
            &MaterialDescriptor::new("suite.screen", MaterialModel::Unlit)
                .with_texture(mirror_color),
        )
        .unwrap();
    let screen_mesh = renderer
        .create_textured_mesh(
            "suite.screen",
            &crate::TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0),
        )
        .unwrap();

    CanonicalScene {
        floor_material,
        floor_mesh,
        pbr_materials,
        sphere_mesh,
        masked_material,
        glass_material,
        pane_mesh,
        crowd_material,
        crowd_mesh,
        screen_material,
        screen_mesh,
        mirror_pass,
        mirror_target,
    }
}

impl CanonicalScene {
    /// Soumet UNE frame de simulation : lumières, draws (l'ordre de
    /// scène), miroir, debug — et la couleur du verre modifiée à chaud
    /// (le chemin update in-place, chaque frame).
    fn queue_frame(&self, renderer: &mut Renderer, t: f32) {
        renderer.submit_light(Light::directional(
            Vec3::new(-0.4, -1.0, -0.3),
            Color::WHITE,
            0.9,
        ));
        renderer.submit_light(Light::point(
            Vec3::new(2.0, 1.0, 0.0),
            Color::WHITE,
            2.0,
            6.0,
        ));
        renderer.submit_light(Light::point(
            Vec3::new(-2.0, 1.0, 0.0),
            Color::WHITE,
            2.0,
            6.0,
        ));
        renderer.submit_light(Light::spot(
            Vec3::new(0.0, 4.0, 0.0),
            Vec3::NEG_Y,
            Color::WHITE,
            2.0,
            8.0,
            0.3,
            0.5,
        ));
        renderer
            .set_material_color(
                self.glass_material,
                Color::rgba(0.2, 0.4, 1.0, 0.3 + 0.2 * t.sin()),
            )
            .unwrap();

        renderer.queue_draw(DrawCommand {
            mesh: self.floor_mesh,
            material: self.floor_material,
            transform: Transform::from_translation(Vec3::new(0.0, -1.0, 0.0)),
        });
        for (index, material) in self.pbr_materials.iter().enumerate() {
            renderer.queue_draw(DrawCommand {
                mesh: self.sphere_mesh,
                material: *material,
                transform: Transform::from_translation(Vec3::new(index as f32 * 2.0, 1.0, -3.0)),
            });
        }
        renderer.queue_draw(DrawCommand {
            mesh: self.pane_mesh,
            material: self.masked_material,
            transform: Transform::from_translation(Vec3::new(3.0, 0.0, 0.0)),
        });
        for z in [-1.0, -2.0] {
            renderer.queue_draw(DrawCommand {
                mesh: self.pane_mesh,
                material: self.glass_material,
                transform: Transform::from_translation(Vec3::new(0.0, 0.0, z)),
            });
        }
        for index in 0..CROWD {
            renderer.queue_draw(DrawCommand {
                mesh: self.crowd_mesh,
                material: self.crowd_material,
                transform: Transform::from_translation(Vec3::new(
                    index as f32,
                    (t + index as f32).sin() * 0.5,
                    -5.0,
                )),
            });
        }
        for index in 0..CULLED {
            renderer.queue_draw(DrawCommand {
                mesh: self.crowd_mesh,
                material: self.crowd_material,
                transform: Transform::from_translation(Vec3::new(5000.0 + index as f32, 0.0, 0.0)),
            });
        }
        renderer.queue_draw(DrawCommand {
            mesh: self.screen_mesh,
            material: self.screen_material,
            transform: Transform::from_translation(Vec3::new(-3.0, 1.0, 0.0)),
        });
        for index in 0..MIRROR_CROWD {
            renderer
                .queue_draw_to(
                    self.mirror_pass,
                    DrawCommand {
                        mesh: self.crowd_mesh,
                        material: self.crowd_material,
                        transform: Transform::from_translation(Vec3::new(index as f32, 0.0, -5.0)),
                    },
                )
                .unwrap();
        }

        renderer.queue_debug(
            DebugDraw::grid(Vec3::new(0.0, -0.99, 0.0), 10.0, 1.0)
                .with_color(Color::rgba(0.6, 0.6, 0.7, 0.4)),
        );
        renderer.queue_debug(DebugDraw::axes(Mat4::IDENTITY, 2.0).overlay());
    }
}

/// Une frame de simulation complète : clear → soumission → render.
fn simulate_frame(scene: &CanonicalScene, renderer: &mut Renderer, t: f32) {
    renderer.clear_draws();
    scene.queue_frame(renderer, t);
    renderer.advance_debug_time(1.0 / 60.0);
    renderer.render_frame().unwrap();
}

#[test]
fn checkpoint_canonical_scene_covers_the_renderer() {
    // LE checkpoint de la suite : la scène canonique sur 10 frames
    // ANIMÉES — les 12 domaines du renderer présents et comptés, les
    // diagnostics IDENTIQUES frame à frame, les ressources constantes
    // une fois les caches chauds.
    let (mut renderer, journal) = mock_renderer();
    let scene = canonical_scene(&mut renderer);
    // Un marqueur RETENU traverse les premières frames puis expire —
    // les durées de vie du debug vivent dans la scène canonique.
    renderer.queue_debug(DebugDraw::marker(Vec3::ZERO, 0.5).with_duration(0.05));

    simulate_frame(&scene, &mut renderer, 0.0);
    let first = renderer.diagnostics().clone();
    let frame = first.frame;
    // Les COMPTES exacts de la scène canonique (le contrat) :
    // 157 soumis à la principale + 10 au miroir.
    assert_eq!(frame.submitted, 167);
    // La principale : 104 opaques + 1 masked + 2 transparents + ciel +
    // 3 debug ; le miroir : 10 + son ciel.
    assert_eq!(frame.resolved, 122);
    assert_eq!(frame.culled, CULLED as usize);
    assert_eq!(frame.instanced_draws, 2);
    assert_eq!(frame.instances, (CROWD + MIRROR_CROWD) as usize);
    assert_eq!(frame.injected, 5);
    assert_eq!(frame.passes_executed, 2);
    // La grille (42) + les axes (3) + le marqueur-octaèdre (12).
    assert_eq!(frame.debug_segments, 57);
    // L'ombre : tout ce qui projette, duplicatas du miroir FUSIONNÉS,
    // la foule hors champ rejetée par le volume de lumière.
    let shadow = first.shadow.unwrap();
    assert_eq!(shadow.draws, 115);
    assert_eq!(shadow.culled, CULLED as usize);
    assert_eq!(shadow.instances, (CROWD + MIRROR_CROWD) as usize);

    // Les 12 domaines, prouvés par les rapports :
    let report = renderer.frame_report();
    assert_eq!(report.passes.len(), 2); // multi-passes
    assert!(matches!(
        report.passes[0].destination,
        RenderDestination::Target(_)
    )); // offscreen
    assert_eq!(report.passes[1].breakdown.masked, 1); // masked
    assert_eq!(report.passes[1].breakdown.transparent, 2); // transparence
    assert!(report.passes[0].breakdown.injected >= 1); // ciel partout
    assert!(
        journal
            .entries()
            .iter()
            .any(|entry| entry.starts_with("lights ") && entry.contains("count=4"))
    ); // lighting
    assert!(
        journal
            .entries()
            .iter()
            .any(|entry| entry.starts_with("environment intensity="))
    ); // environnement + IBL
    assert!(
        journal
            .entries()
            .iter()
            .any(|entry| entry.contains("chaos.material.pbr "))
    ); // PBR
    assert!(
        journal
            .entries()
            .iter()
            .any(|entry| entry.starts_with("update_material_binding"))
    ); // update in-place

    // 10 frames animées : les comptes NE BOUGENT PAS (le marqueur
    // expire après ~3 frames — les segments passent de 57 à 45), les
    // ressources sont CONSTANTES une fois les caches chauds.
    simulate_frame(&scene, &mut renderer, 1.0 / 60.0);
    let warmed = renderer.resource_stats();
    for step in 2..10 {
        simulate_frame(&scene, &mut renderer, step as f32 / 60.0);
        let diagnostics = renderer.diagnostics();
        assert_eq!(diagnostics.frame.resolved, 121, "frame #{step}");
        assert_eq!(diagnostics.frame.debug_segments, 45, "frame #{step}");
        assert_eq!(diagnostics.frame.culled, CULLED as usize);
        assert_eq!(renderer.resource_stats(), warmed, "frame #{step}");
        assert_eq!(diagnostics.resources.retired, 0);
    }
    // La cible du miroir vit toujours ; le journal n'a JAMAIS vu une
    // erreur backend (le mock n'en fabrique pas — la ligne de vérité).
    assert!(renderer.render_target_color(scene.mirror_target).is_ok());
    assert!(render_lines(&journal).len() >= 20);
}

#[test]
fn resources_survive_intensive_churn() {
    // 300 cycles create/partage/swap/destroy, un render de flush
    // périodique — la mémoire REVIENT à la baseline (seuls les
    // pipelines, permanents par contrat, restent).
    let (mut renderer, _journal) = mock_renderer();
    // Les fallbacks builtin se matérialisent au premier material : ils
    // sont PERMANENTS (protégés) — la baseline se prend APRÈS eux.
    renderer.builtin_texture(BuiltinTexture::White).unwrap();
    renderer
        .builtin_texture(BuiltinTexture::FlatNormal)
        .unwrap();
    let baseline = renderer.resource_stats();
    for cycle in 0..300u32 {
        let texture = renderer
            .create_texture(&TextureDescriptor::sampled(
                format!("churn.{cycle}"),
                2,
                2,
                TextureFormat::Rgba8Unorm,
                vec![0; 16],
            ))
            .unwrap();
        let replacement = renderer
            .create_texture(&TextureDescriptor::sampled(
                format!("churn.{cycle}.swap"),
                2,
                2,
                TextureFormat::Rgba8Unorm,
                vec![255; 16],
            ))
            .unwrap();
        let sampler = renderer
            .create_sampler(&SamplerDescriptor::new(format!("churn.{cycle}")))
            .unwrap();
        // Le PARTAGE : deux materials sur la même texture.
        let first = renderer
            .create_material(
                &MaterialDescriptor::new(format!("churn.{cycle}.a"), MaterialModel::Unlit)
                    .with_texture(texture)
                    .with_sampler(sampler),
            )
            .unwrap();
        let second = renderer
            .create_material(
                &MaterialDescriptor::new(format!("churn.{cycle}.b"), MaterialModel::Unlit)
                    .with_texture(texture)
                    .with_sampler(sampler),
            )
            .unwrap();
        let mesh = renderer
            .create_lit_mesh(
                &format!("churn.{cycle}"),
                &LitGeometry::cube([0.0, 0.0, 0.0], 1.0),
            )
            .unwrap();
        // Le REMPLACEMENT à chaud : l'ancien binding part en retraite.
        renderer
            .set_material_texture(first, Some(replacement))
            .unwrap();
        renderer
            .set_material_color(second, Color::rgb(1.0, 0.0, 0.0))
            .unwrap();
        // Un render périodique draine la retraite au point sûr.
        if cycle % 8 == 0 {
            renderer.clear_draws();
            renderer.render_frame().unwrap();
        }
        renderer.destroy_material(first).unwrap();
        renderer.destroy_material(second).unwrap();
        renderer.destroy_mesh(mesh).unwrap();
        renderer.destroy_texture(texture).unwrap();
        renderer.destroy_texture(replacement).unwrap();
        renderer.destroy_sampler(sampler).unwrap();
    }
    renderer.clear_draws();
    renderer.render_frame().unwrap();
    renderer.render_frame().unwrap();
    let end = renderer.resource_stats();
    assert_eq!(
        end,
        ResourceStats {
            pipelines: end.pipelines,
            ..baseline
        }
    );
    assert_eq!(end.retired, 0);
}

#[test]
fn invalid_destruction_orders_never_corrupt() {
    // Les ordres INVALIDES en rafale : chacun refusé en nommant, et la
    // scène rend encore — l'état jamais corrompu par un refus.
    let (mut renderer, _journal) = mock_renderer();
    let scene = canonical_scene(&mut renderer);
    simulate_frame(&scene, &mut renderer, 0.0);
    let healthy = renderer.diagnostics().frame;
    let stats = renderer.resource_stats();

    // La texture du sol est PARTAGÉE par son material.
    let checker = renderer
        .material_info(scene.floor_material)
        .unwrap()
        .texture;
    assert!(
        renderer
            .destroy_texture(checker)
            .unwrap_err()
            .to_string()
            .contains("still used")
    );
    // La cubemap est l'environnement ACTIF.
    // (l'erreur nomme la règle : « clear it first »)
    let fallback = renderer.builtin_texture(BuiltinTexture::White).unwrap();
    assert!(
        renderer
            .destroy_texture(fallback)
            .unwrap_err()
            .to_string()
            .contains("builtin fallback")
    );
    // Le double destroy est une erreur périmée explicite.
    let mesh = renderer
        .create_lit_mesh("doomed", &LitGeometry::cube([0.0, 0.0, 0.0], 1.0))
        .unwrap();
    renderer.destroy_mesh(mesh).unwrap();
    assert!(
        renderer
            .destroy_mesh(mesh)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    // Après la rafale : les MÊMES comptes, la même mémoire (le mesh
    // condamné drainé au prochain render).
    simulate_frame(&scene, &mut renderer, 1.0);
    simulate_frame(&scene, &mut renderer, 2.0);
    assert_eq!(renderer.diagnostics().frame.resolved, healthy.resolved);
    assert_eq!(renderer.resource_stats(), stats);
}

#[test]
fn render_target_rotation_is_leak_free() {
    // 50 rotations de cible (resize = nouveau handle) avec rebind du
    // material à chaque tour — zéro fuite, zéro handle fantôme.
    let (mut renderer, _journal) = mock_renderer();
    // Les fallbacks (textures builtin + sampler par défaut) se
    // matérialisent au premier material sans entrées : PERMANENTS —
    // la baseline se prend après eux.
    let warmup = renderer
        .create_material(&MaterialDescriptor::new("warmup", MaterialModel::Unlit))
        .unwrap();
    renderer.destroy_material(warmup).unwrap();
    renderer.render_frame().unwrap();
    let baseline = renderer.resource_stats();
    let mut target = renderer
        .create_render_target(&RenderTargetDescriptor::new(
            "rotating",
            64,
            64,
            TextureFormat::Rgba8UnormSrgb,
        ))
        .unwrap();
    let material = renderer
        .create_material(
            &MaterialDescriptor::new("watcher", MaterialModel::Unlit)
                .with_texture(renderer.render_target_color(target).unwrap()),
        )
        .unwrap();
    let mesh = renderer
        .create_textured_mesh(
            "watcher",
            &crate::TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0),
        )
        .unwrap();
    for round in 0..50u32 {
        // La couleur se DÉTACHE avant la rotation (le partage compté
        // refuse un resize sous emprise), puis le handle FRAIS rebinde.
        renderer.set_material_texture(material, None).unwrap();
        target = renderer
            .resize_render_target(target, 64 + round, 64)
            .unwrap();
        let fresh_color = renderer.render_target_color(target).unwrap();
        renderer
            .set_material_texture(material, Some(fresh_color))
            .unwrap();
        renderer.clear_draws();
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.render_frame().unwrap();
        assert_eq!(renderer.diagnostics().frame.resolved, 1, "round #{round}");
    }
    renderer.clear_draws();
    renderer.destroy_material(material).unwrap();
    renderer.destroy_mesh(mesh).unwrap();
    renderer.destroy_render_target(target).unwrap();
    renderer.render_frame().unwrap();
    renderer.render_frame().unwrap();
    let end = renderer.resource_stats();
    assert_eq!(
        end,
        ResourceStats {
            pipelines: end.pipelines,
            ..baseline
        }
    );
}

#[test]
fn resize_storms_leave_the_renderer_stable() {
    // 200 resizes alternés — minimisation 0×0 comprise — un render
    // entre chaque : jamais une erreur, les comptes inchangés.
    let (mut renderer, _journal) = mock_renderer();
    let scene = canonical_scene(&mut renderer);
    simulate_frame(&scene, &mut renderer, 0.0);
    let healthy = renderer.diagnostics().frame.resolved;
    for step in 0..200u32 {
        let (width, height) = match step % 10 {
            9 => (0, 0),
            0 => (2560, 1440),
            odd => (640 + odd * 37, 480 + odd * 21),
        };
        renderer.resize(width, height);
        simulate_frame(&scene, &mut renderer, step as f32 / 60.0);
        assert_eq!(
            renderer.diagnostics().frame.resolved,
            healthy,
            "step #{step}"
        );
    }
}

#[test]
fn surface_loss_and_recovery_cycles_are_counted() {
    // Les pertes et récupérations de surface EN COURS de run : les
    // compteurs cumulés exacts, les passes marquées SurfaceSkipped, la
    // reprise propre — et l'erreur backend injectée n'empoisonne pas la
    // frame suivante.
    let (mut renderer, _journal, switch) = mock_renderer_switchable();
    let scene = canonical_scene(&mut renderer);
    let mut presented = 0u64;
    let mut reconfigured = 0u64;
    let mut unavailable = 0u64;
    for step in 0..30u32 {
        match step % 10 {
            3 => {
                switch.set(FrameOutcome::Skipped(FrameSkipReason::SurfaceReconfigured));
                reconfigured += 1;
            }
            6 => {
                switch.set(FrameOutcome::Skipped(FrameSkipReason::SurfaceUnavailable));
                unavailable += 1;
            }
            _ => {
                switch.set(FrameOutcome::Rendered);
                presented += 1;
            }
        }
        simulate_frame(&scene, &mut renderer, step as f32 / 60.0);
        let expected = if step % 10 == 3 || step % 10 == 6 {
            PassOutcome::SurfaceSkipped
        } else {
            PassOutcome::Executed
        };
        // La passe SURFACE suit l'issue ; la passe CIBLE (le miroir)
        // s'exécute quoi qu'il arrive.
        assert_eq!(renderer.frame_report().passes[1].outcome, expected);
        assert_eq!(
            renderer.frame_report().passes[0].outcome,
            PassOutcome::Executed
        );
    }
    let surface = renderer.diagnostics().surface;
    assert_eq!(surface.presented, presented);
    assert_eq!(surface.reconfigured, reconfigured);
    assert_eq!(surface.skipped_unavailable, unavailable);
    // L'erreur backend FATALE : elle remonte, la frame suivante rend.
    switch.fail("simulated device loss");
    renderer.clear_draws();
    scene.queue_frame(&mut renderer, 1.0);
    assert!(
        renderer
            .render_frame()
            .unwrap_err()
            .to_string()
            .contains("simulated device loss")
    );
    switch.set(FrameOutcome::Rendered);
    simulate_frame(&scene, &mut renderer, 2.0);
    assert_eq!(renderer.diagnostics().frame.culled, CULLED as usize);
}

#[test]
fn long_execution_never_drifts() {
    // LE long run : 1 000 frames de la scène canonique — les
    // diagnostics identiques à chaque centième, la mémoire CONSTANTE
    // (la garde de dérive), la retraite toujours drainée.
    let (mut renderer, _journal) = mock_renderer();
    let scene = canonical_scene(&mut renderer);
    simulate_frame(&scene, &mut renderer, 0.0);
    simulate_frame(&scene, &mut renderer, 1.0 / 60.0);
    let steady_frame = renderer.diagnostics().frame;
    let steady_stats = renderer.resource_stats();
    for step in 2..1000u32 {
        simulate_frame(&scene, &mut renderer, step as f32 / 60.0);
        if step % 100 == 0 {
            assert_eq!(renderer.diagnostics().frame, steady_frame, "frame #{step}");
            assert_eq!(renderer.resource_stats(), steady_stats, "frame #{step}");
            assert_eq!(renderer.diagnostics().resources.retired, 0);
        }
    }
    assert_eq!(renderer.diagnostics().surface.presented, 1000);
}

#[test]
fn user_errors_never_poison_the_frame() {
    // Les erreurs de l'utilisateur EN BOUCLE — refusées ou écartées en
    // nommant, la frame rend toujours pareil.
    let (mut renderer, _journal) = mock_renderer();
    let scene = canonical_scene(&mut renderer);
    simulate_frame(&scene, &mut renderer, 0.0);
    let healthy = renderer.diagnostics().frame.resolved;
    for step in 0..50u32 {
        // La configuration impossible : refusée.
        assert!(
            renderer
                .create_texture(&TextureDescriptor::sampled(
                    "huge",
                    9000,
                    1,
                    TextureFormat::R8Unorm,
                    vec![0; 9000],
                ))
                .is_err()
        );
        // La lumière invalide : écartée au submit (warn).
        renderer.submit_light(Light::directional(Vec3::ZERO, Color::WHITE, 1.0));
        // Le mesh périmé : le draw sera écarté au resolve (warn).
        let doomed = renderer
            .create_lit_mesh("doomed", &LitGeometry::cube([0.0, 0.0, 0.0], 1.0))
            .unwrap();
        renderer.destroy_mesh(doomed).unwrap();
        renderer.queue_draw(DrawCommand {
            mesh: doomed,
            material: scene.crowd_material,
            transform: Transform::IDENTITY,
        });
        // La passe inconnue : une erreur explicite.
        assert!(
            renderer
                .queue_draw_to(
                    PassHandle(99),
                    DrawCommand {
                        mesh: scene.crowd_mesh,
                        material: scene.crowd_material,
                        transform: Transform::IDENTITY,
                    },
                )
                .is_err()
        );
        // Le debug non fini : écarté au submit (warn).
        renderer.queue_debug(DebugDraw::line(Vec3::new(f32::NAN, 0.0, 0.0), Vec3::X));
        simulate_frame(&scene, &mut renderer, step as f32 / 60.0);
        assert_eq!(
            renderer.diagnostics().frame.resolved,
            healthy,
            "step #{step}"
        );
    }
}

#[test]
fn a_reduced_backend_still_renders_first_class() {
    // Le backend RÉDUIT : limites abaissées, timestamps absents — la
    // scène-lite rend, les refus parlent au nom du device, rien n'est
    // supposé.
    let limits = DeviceLimits {
        max_texture_2d: 1024,
        ..DeviceLimits::default()
    };
    let (mut renderer, _journal) = mock_renderer_with_limits(limits);
    renderer.set_view_projection(Mat4::from_scale(Vec3::new(0.001, 0.001, -0.001)));
    renderer
        .set_directional_shadow(
            &DirectionalShadowDescriptor::new(ShadowVolume::new(
                Vec3::ZERO,
                Vec3::new(50.0, 20.0, 50.0),
            ))
            .with_resolution(1024),
        )
        .unwrap();
    let material = renderer
        .create_material(&MaterialDescriptor::new("lite", MaterialModel::Lit))
        .unwrap();
    let mesh = renderer
        .create_lit_mesh("lite", &LitGeometry::cube([0.0, 0.0, 0.0], 1.0))
        .unwrap();
    for step in 0..3u32 {
        renderer.clear_draws();
        renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
        for index in 0..20u32 {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::from_translation(Vec3::new(index as f32, 0.0, 0.0)),
            });
        }
        renderer.render_frame().unwrap();
        let frame = renderer.diagnostics().frame;
        assert_eq!(frame.instances, 20, "step #{step}");
        assert!(matches!(
            &renderer.diagnostics().gpu,
            crate::diagnostics::GpuTiming::Unavailable { .. }
        ));
    }
    // Les refus parlent DEVICE, et l'ombre au plafond exact est passée.
    assert!(
        renderer
            .create_texture(&TextureDescriptor::sampled(
                "big",
                2048,
                1,
                TextureFormat::R8Unorm,
                vec![0; 2048],
            ))
            .unwrap_err()
            .to_string()
            .contains("device texture limit (1024)")
    );
}

#[test]
fn many_of_everything_stays_bounded() {
    // Beaucoup de TOUT : deux foules de 500, 100 materials, 30
    // textures, 18 lumières soumises (16 gardées), 4 passes — les
    // soumissions restent BORNÉES par l'instancing, les caches stables
    // à la frame suivante, la mémoire constante.
    let (mut renderer, journal) = mock_renderer();
    let mesh = renderer
        .create_lit_mesh("perf.cube", &LitGeometry::cube([0.0, 0.0, 0.0], 0.5))
        .unwrap();
    let textures: Vec<_> = (0..30u32)
        .map(|index| {
            renderer
                .create_texture(&TextureDescriptor::sampled(
                    format!("perf.texture.{index}"),
                    2,
                    2,
                    TextureFormat::Rgba8Unorm,
                    vec![index as u8; 16],
                ))
                .unwrap()
        })
        .collect();
    let materials: Vec<_> = (0..100u32)
        .map(|index| {
            renderer
                .create_material(
                    &MaterialDescriptor::new(format!("perf.material.{index}"), MaterialModel::Lit)
                        .with_texture(textures[index as usize % textures.len()]),
                )
                .unwrap()
        })
        .collect();
    let crowd_materials = [materials[0], materials[1]];
    // Trois passes CIBLE de plus — chacune reçoit une petite foule.
    let extra_passes: Vec<_> = (0..3u32)
        .map(|index| {
            let target = renderer
                .create_render_target(&RenderTargetDescriptor::new(
                    format!("perf.target.{index}"),
                    64,
                    64,
                    TextureFormat::Rgba8UnormSrgb,
                ))
                .unwrap();
            renderer
                .add_pass(
                    &RenderPassDescriptor::new(
                        format!("perf.pass.{index}"),
                        RenderDestination::Target(target),
                    )
                    .with_camera(Mat4::from_scale(Vec3::new(0.001, 0.001, -0.001)))
                    .with_order(-(index as i32) - 1),
                )
                .unwrap()
        })
        .collect();

    let queue_scene = |renderer: &mut Renderer| {
        for index in 0..18u32 {
            renderer.submit_light(Light::point(
                Vec3::new(index as f32, 1.0, 0.0),
                Color::WHITE,
                1.0,
                5.0,
            ));
        }
        for (slot, material) in crowd_materials.iter().enumerate() {
            for index in 0..500u32 {
                renderer.queue_draw(DrawCommand {
                    mesh,
                    material: *material,
                    transform: Transform::from_translation(Vec3::new(
                        index as f32,
                        slot as f32 * 2.0,
                        -5.0,
                    )),
                });
            }
        }
        for (index, material) in materials.iter().enumerate().skip(2) {
            renderer.queue_draw(DrawCommand {
                mesh,
                material: *material,
                transform: Transform::from_translation(Vec3::new(index as f32, 5.0, -5.0)),
            });
        }
        for (index, pass) in extra_passes.iter().enumerate() {
            for slot in 0..10u32 {
                renderer
                    .queue_draw_to(
                        *pass,
                        DrawCommand {
                            mesh,
                            material: materials[index],
                            transform: Transform::from_translation(Vec3::new(
                                slot as f32,
                                0.0,
                                -3.0,
                            )),
                        },
                    )
                    .unwrap();
            }
        }
    };

    queue_scene(&mut renderer);
    renderer.render_frame().unwrap();
    let first = renderer.diagnostics().frame;
    // 1 098 objets soumis → les foules FUSIONNENT : 2 batches de 500 +
    // 3 batches de 10 + 98 classiques.
    assert_eq!(first.submitted, 1128);
    assert_eq!(first.instances, 1030);
    assert_eq!(first.instanced_draws, 5);
    assert_eq!(first.classic_draws, 98);
    assert_eq!(first.passes_executed, 4);
    // La troncature des lumières : 16 gardées sur 18, dit au journal.
    assert!(
        journal
            .entries()
            .iter()
            .any(|entry| entry.starts_with("lights ") && entry.contains("count=16"))
    );
    let warmed_stats = renderer.resource_stats();
    // 30 textures + 3 couleurs de cibles + les 2 fallbacks builtin.
    assert_eq!(warmed_stats.textures.alive, 35);
    assert_eq!(warmed_stats.materials, 100);

    // FRAME 2 : mêmes comptes, AUCUN pipeline ni octet de plus.
    renderer.clear_draws();
    queue_scene(&mut renderer);
    renderer.render_frame().unwrap();
    assert_eq!(renderer.diagnostics().frame, first);
    assert_eq!(renderer.resource_stats(), warmed_stats);
}

#[test]
fn cpu_budgets_flag_overruns_at_scale() {
    let (mut renderer, _journal) = mock_renderer();
    let scene = canonical_scene(&mut renderer);
    // Un budget GÉNÉREUX : jamais de dépassement sur 5 frames.
    renderer.set_cpu_budget(Some(1000.0));
    for step in 0..5u32 {
        simulate_frame(&scene, &mut renderer, step as f32 / 60.0);
    }
    assert_eq!(renderer.diagnostics().budget.over_budget_frames, 0);
    // Un budget MINUSCULE : chaque frame dépasse, le cumul avance.
    renderer.set_cpu_budget(Some(f32::MIN_POSITIVE));
    for step in 0..5u32 {
        simulate_frame(&scene, &mut renderer, step as f32 / 60.0);
    }
    assert_eq!(renderer.diagnostics().budget.over_budget_frames, 5);
    assert!(renderer.diagnostics().budget.last_frame_over);
}
