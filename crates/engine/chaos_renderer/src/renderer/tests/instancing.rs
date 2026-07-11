//! L'instancing : la fusion des runs compatibles, ses exclusions
//! (transparents, catégories mélangées) et le checkpoint V1.

use super::*;

#[test]
fn transparents_are_never_instanced() {
    let (mut renderer, journal) = mock_renderer();
    let glass = renderer
        .create_material(
            &MaterialDescriptor::new("glass", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
    let mesh = lit_quad_mesh(&mut renderer, "pane");
    for z in [-1.0, -2.0, -3.0] {
        renderer.queue_draw(DrawCommand {
            mesh,
            material: glass,
            transform: Transform::from_translation(Vec3::new(0.0, 0.0, z)),
        });
    }
    renderer.render_frame().unwrap();
    // Trois draws individuels, triés par profondeur — jamais un
    // `inst=` : le tri des transparents prime sur le regroupement.
    let line = render_lines(&journal).pop().unwrap();
    assert_eq!(line.matches("m=[").count(), 3);
    assert!(!line.contains("inst="));
    let report = renderer.frame_report();
    assert_eq!(report.passes[0].draws, 3);
    assert_eq!(report.passes[0].draw_calls, 3);
}

#[test]
fn masked_and_opaque_runs_never_share_a_batch() {
    let (mut renderer, journal) = mock_renderer();
    let solid = renderer
        .create_material(&MaterialDescriptor::new("solid", MaterialModel::Lit))
        .unwrap();
    let grid = renderer
        .create_material(
            &MaterialDescriptor::new("grid", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Masked),
        )
        .unwrap();
    let mesh = lit_quad_mesh(&mut renderer, "pane");
    for material in [grid, solid, grid, solid] {
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
    }
    renderer.render_frame().unwrap();
    // DEUX batches de 2 : l'opaque sur sa permutation `.instanced`,
    // le masked sur `.masked.instanced` (entrées vs_instanced +
    // fs_masked) — les catégories ne se mélangent jamais.
    let line = render_lines(&journal).pop().unwrap();
    assert_eq!(line.matches("inst=2").count(), 2);
    let instanced_pipelines: Vec<String> = create_pipeline_lines(&journal)
        .into_iter()
        .filter(|line| line.contains(" instanced"))
        .collect();
    assert_eq!(instanced_pipelines.len(), 2);
    assert!(
        instanced_pipelines
            .iter()
            .any(|line| line.starts_with("create_pipeline chaos.material.lit.instanced "))
    );
    assert!(instanced_pipelines.iter().any(|line| {
        line.starts_with("create_pipeline chaos.material.lit.masked.instanced ")
            && line.contains(" entry=fs_masked")
    }));
    assert_eq!(
        renderer.frame_report().passes[0].breakdown,
        DrawBreakdown {
            opaque: 2,
            masked: 2,
            transparent: 0,
            injected: 0,
        }
    );
    assert_eq!(renderer.frame_report().passes[0].draws, 4);
    assert_eq!(renderer.frame_report().passes[0].draw_calls, 2);
}

#[test]
fn render_to_target_batches_too() {
    let (mut renderer, journal) = mock_renderer();
    let solid = renderer
        .create_material(&MaterialDescriptor::new("solid", MaterialModel::Lit))
        .unwrap();
    let mesh = lit_quad_mesh(&mut renderer, "pane");
    let target = renderer
        .create_render_target(&RenderTargetDescriptor::new(
            "vignette",
            64,
            64,
            TextureFormat::Rgba8UnormSrgb,
        ))
        .unwrap();
    let command = DrawCommand {
        mesh,
        material: solid,
        transform: Transform::IDENTITY,
    };
    renderer
        .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[command, command])
        .unwrap();
    let line = render_lines(&journal).pop().unwrap();
    assert_eq!(line.matches("m=[").count(), 1);
    assert!(line.contains("inst=2"));
    // La permutation instanciée VISE le format de la cible — le
    // descripteur, pas seulement le label (le run GPU l'exige).
    assert!(
        create_pipeline_lines(&journal)
            .iter()
            .any(|line| { line.contains(".instanced") && line.contains(" target=Rgba8UnormSrgb") })
    );
}

#[test]
fn instanced_shadow_casters_report_their_draw_calls() {
    let (mut renderer, journal) = mock_renderer();
    renderer.set_directional_shadow(&demo_shadow()).unwrap();
    renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
    let caster = lit_caster(&mut renderer, "caster");
    for x in 0u8..5 {
        renderer.queue_draw(DrawCommand {
            transform: Transform::from_translation(Vec3::new(f32::from(x), 0.0, 0.0)),
            ..caster
        });
    }
    renderer.render_frame().unwrap();
    // Cinq casters d'un même (material, mesh) = UN draw d'ombre
    // instancié — le rapport dit les objets ET les soumissions.
    assert_eq!(
        renderer.frame_report().shadow,
        Some(ShadowReport {
            draws: 5,
            draw_calls: 1,
            culled: 0,
            resolution: 2048
        })
    );
    let shadow = shadow_lines(&journal).pop().unwrap();
    assert_eq!(shadow.matches("m=[").count(), 1);
    assert!(shadow.contains("inst=5"));
}

#[test]
fn checkpoint_instancing_v1_a_crowd_collapses_to_a_few_draw_calls() {
    // LE checkpoint Instancing V1 : 500 objets compatibles + des
    // incompatibles mélangés — les draw calls tombent d'un ordre de
    // grandeur, les incompatibles restent des draws classiques, les
    // ombres profitent pareil, et le consommateur n'a rien changé
    // (il soumet toujours objet par objet).
    let (mut renderer, journal) = mock_renderer();
    // Le volume d'ombre couvre TOUTE la foule (x 0..499) : le
    // culling d'ombre ne rejette rien ici — il a son propre test.
    renderer
        .set_directional_shadow(&DirectionalShadowDescriptor::new(ShadowVolume::new(
            Vec3::new(250.0, 0.0, 0.0),
            Vec3::new(300.0, 20.0, 300.0),
        )))
        .unwrap();
    let crowd = lit_caster(&mut renderer, "crowd");
    let loner = lit_caster(&mut renderer, "loner");
    let glass = renderer
        .create_material(
            &MaterialDescriptor::new("glass", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
    let queue_scene = |renderer: &mut Renderer| {
        renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 0.9));
        for index in 0..500u32 {
            renderer.queue_draw(DrawCommand {
                transform: Transform::from_translation(Vec3::new(index as f32, 0.0, 0.0)),
                ..crowd
            });
        }
        renderer.queue_draw(loner);
        for z in [-1.0, -2.0, -3.0] {
            renderer.queue_draw(DrawCommand {
                mesh: loner.mesh,
                material: glass,
                transform: Transform::from_translation(Vec3::new(0.0, 0.0, z)),
            });
        }
    };

    // FRAME 1 : 504 objets logiques → 5 soumissions (la foule en 1,
    // le solitaire en 1, les 3 transparents individuels).
    queue_scene(&mut renderer);
    renderer.render_frame().unwrap();
    let report = renderer.frame_report();
    assert_eq!(report.passes[0].draws, 504);
    assert_eq!(report.passes[0].draw_calls, 5);
    // Les ombres profitent pareil : 501 casters (les transparents
    // jamais) → 2 soumissions.
    assert_eq!(
        report.shadow,
        Some(ShadowReport {
            draws: 501,
            draw_calls: 2,
            culled: 0,
            resolution: 2048
        })
    );
    let line = render_lines(&journal).pop().unwrap();
    assert!(line.contains("inst=500"));

    // FRAME 2 : mêmes soumissions — AUCUN pipeline de plus (le
    // cache des permutations instanciées tient), mêmes comptes.
    let pipelines_before = create_pipeline_lines(&journal).len();
    renderer.clear_draws();
    queue_scene(&mut renderer);
    renderer.render_frame().unwrap();
    assert_eq!(create_pipeline_lines(&journal).len(), pipelines_before);
    assert_eq!(renderer.frame_report().passes[0].draw_calls, 5);
}
