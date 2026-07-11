//! Le frustum culling : rejets conservateurs par passe, exemptions
//! (bounds absents, opt-out), frustum de la lumière et checkpoint V1.

use super::*;

#[test]
fn an_out_of_view_draw_is_culled_from_the_pass() {
    let (mut renderer, journal) = mock_renderer();
    let material = plain_material(&mut renderer, "p");
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    renderer.queue_draw(DrawCommand {
        mesh,
        material,
        transform: Transform::from_translation(Vec3::new(5000.0, 0.0, 0.0)),
    });
    renderer.render_frame().unwrap();
    // Hors du champ de la caméra large (±1000) : jamais résolu,
    // jamais soumis — le plan part VIDE, le rapport dit pourquoi.
    let line = render_lines(&journal).pop().unwrap();
    assert!(line.ends_with("vp=[0, 0, 0] draws=[]"));
    let report = &renderer.frame_report().passes[0];
    assert_eq!(report.draws, 0);
    assert_eq!(report.draw_calls, 0);
    assert_eq!(report.culled, 1);
    assert_eq!(report.breakdown, DrawBreakdown::default());
}

#[test]
fn a_straddling_draw_is_never_wrongly_rejected() {
    let (mut renderer, journal) = mock_renderer();
    let material = plain_material(&mut renderer, "p");
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    // À cheval sur le bord droit du frustum (x = 1000, bounds
    // ±0.5) : partiellement visible → JAMAIS rejeté — le
    // conservatisme est le contrat.
    renderer.queue_draw(DrawCommand {
        mesh,
        material,
        transform: Transform::from_translation(Vec3::new(1000.0, 0.0, 0.0)),
    });
    renderer.render_frame().unwrap();
    let line = render_lines(&journal).pop().unwrap();
    assert!(line.contains("m=[1000, 0, 0]"));
    assert_eq!(renderer.frame_report().passes[0].culled, 0);
}

#[test]
fn a_boundless_mesh_is_never_culled() {
    let (mut renderer, journal) = mock_renderer();
    let material = plain_material(&mut renderer, "p");
    // Une position non finie refuse les bounds à la création : le
    // mesh n'en a pas — le défaut SÛR le dessine toujours, même
    // très loin hors champ.
    let mesh = renderer
        .create_mesh(
            "broken",
            &Geometry {
                vertices: vec![
                    ColorVertex {
                        position: [f32::NAN, 0.0, 0.0],
                        color: [1.0, 1.0, 1.0],
                    },
                    ColorVertex {
                        position: [1.0, 0.0, 0.0],
                        color: [1.0, 1.0, 1.0],
                    },
                    ColorVertex {
                        position: [0.0, 1.0, 0.0],
                        color: [1.0, 1.0, 1.0],
                    },
                ],
                indices: Vec::new(),
            },
        )
        .unwrap();
    renderer.queue_draw(DrawCommand {
        mesh,
        material,
        transform: Transform::from_translation(Vec3::new(5000.0, 0.0, 0.0)),
    });
    renderer.render_frame().unwrap();
    let line = render_lines(&journal).pop().unwrap();
    assert!(line.contains("m=[5000, 0, 0]"));
    assert_eq!(renderer.frame_report().passes[0].culled, 0);
}

#[test]
fn an_unculled_material_ignores_every_frustum() {
    let (mut renderer, journal) = mock_renderer();
    renderer.set_directional_shadow(&demo_shadow()).unwrap();
    renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
    let ghost = renderer
        .create_material(
            &MaterialDescriptor::new("ghost", MaterialModel::Lit).without_frustum_culling(),
        )
        .unwrap();
    let mesh = lit_quad_mesh(&mut renderer, "pane");
    // Hors caméra (±1000) ET hors volume de lumière (±10) : le
    // forcé-visible saute les DEUX tests — dessiné ET moissonné.
    renderer.queue_draw(DrawCommand {
        mesh,
        material: ghost,
        transform: Transform::from_translation(Vec3::new(5000.0, 0.0, 0.0)),
    });
    renderer.render_frame().unwrap();
    assert!(
        render_lines(&journal)
            .pop()
            .unwrap()
            .contains("m=[5000, 0, 0]")
    );
    assert!(
        shadow_lines(&journal)
            .pop()
            .unwrap()
            .contains("m=[5000, 0, 0]")
    );
    assert_eq!(renderer.frame_report().passes[0].culled, 0);
    assert_eq!(renderer.frame_report().shadow.unwrap().culled, 0);
}

#[test]
fn transparents_are_culled_before_the_sort() {
    let (mut renderer, journal) = mock_renderer();
    let glass = renderer
        .create_material(
            &MaterialDescriptor::new("glass", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
    let mesh = lit_quad_mesh(&mut renderer, "pane");
    for translation in [
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::new(0.0, 0.0, -2.0),
        Vec3::new(5000.0, 0.0, 0.0),
    ] {
        renderer.queue_draw(DrawCommand {
            mesh,
            material: glass,
            transform: Transform::from_translation(translation),
        });
    }
    renderer.render_frame().unwrap();
    // Le hors-champ sort AVANT le tri arrière → avant : deux
    // transparents triés, le troisième compté cullé.
    let line = render_lines(&journal).pop().unwrap();
    assert_eq!(line.matches("m=[").count(), 2);
    assert!(!line.contains("5000"));
    let report = &renderer.frame_report().passes[0];
    assert_eq!(report.breakdown.transparent, 2);
    assert_eq!(report.culled, 1);
}

#[test]
fn instancing_only_fuses_the_visible() {
    let (mut renderer, journal) = mock_renderer();
    let material = plain_material(&mut renderer, "crowd");
    let mesh = renderer.create_mesh("cube", &cube()).unwrap();
    // Dix objets compatibles, un sur deux hors champ (x = 0..2250,
    // la caméra large s'arrête à ±1000) : le run instancié ne
    // contient QUE les visibles — le culling se joue AVANT la
    // fusion.
    for index in 0..10u32 {
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::from_translation(Vec3::new(250.0 * index as f32, 0.0, 0.0)),
        });
    }
    renderer.render_frame().unwrap();
    let line = render_lines(&journal).pop().unwrap();
    assert_eq!(line.matches("m=[").count(), 1);
    assert!(line.contains("inst=5"));
    let report = &renderer.frame_report().passes[0];
    assert_eq!(report.draws, 5);
    assert_eq!(report.draw_calls, 1);
    assert_eq!(report.culled, 5);
}

#[test]
fn a_caster_off_screen_keeps_its_shadow_and_vice_versa() {
    // L'ANTI-POP : la moisson d'ombre teste le frustum de la
    // LUMIÈRE, jamais celui de la passe — un caster sorti de
    // l'écran projette encore, un visible hors volume ne projette
    // plus (et son ombre au sol disparaît avec le volume, pas avec
    // la caméra).
    let (mut renderer, journal) = mock_renderer();
    // Une caméra étroite : le cube NDC décalé — visible x ∈ [4, 6].
    renderer.set_view_projection(Mat4::from_translation(Vec3::new(-5.0, 0.0, 0.0)));
    renderer
        .set_directional_shadow(&DirectionalShadowDescriptor::new(ShadowVolume::new(
            Vec3::ZERO,
            Vec3::new(2.0, 2.0, 2.0),
        )))
        .unwrap();
    renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
    let hidden = lit_caster(&mut renderer, "hidden");
    let lonely = lit_caster(&mut renderer, "lonely");
    renderer.queue_draw(hidden);
    renderer.queue_draw(DrawCommand {
        transform: Transform::from_translation(Vec3::new(5.0, 0.0, 0.5)),
        ..lonely
    });
    renderer.render_frame().unwrap();
    // `hidden` (origine) : hors caméra, dans le volume — ABSENT de
    // la passe, PRÉSENT dans l'ombre.
    let render = render_lines(&journal).pop().unwrap();
    assert!(render.contains("m=[5, 0, 0.5]"));
    assert!(!render.contains("m=[0, 0, 0]"));
    // `lonely` (x = 5) : visible, hors volume — l'inverse.
    let shadow = shadow_lines(&journal).pop().unwrap();
    assert!(shadow.contains("m=[0, 0, 0]"));
    assert!(!shadow.contains("m=[5, 0, 0.5]"));
    let report = renderer.frame_report();
    assert_eq!(report.passes[0].draws, 1);
    assert_eq!(report.passes[0].culled, 1);
    assert_eq!(
        report.shadow,
        Some(ShadowReport {
            draws: 1,
            draw_calls: 1,
            culled: 1,
            resolution: 2048
        })
    );
}

#[test]
fn each_view_culls_with_its_own_frustum() {
    let (mut renderer, journal) = mock_renderer();
    let overlay = renderer.add_pass(&surface_pass("overlay")).unwrap();
    renderer
        .set_pass_camera(overlay, Mat4::from_translation(Vec3::new(-5.0, 0.0, 0.0)))
        .unwrap();
    let material = plain_material(&mut renderer, "p");
    let mesh = renderer.create_mesh("cube", &cube()).unwrap();
    let near = DrawCommand {
        mesh,
        material,
        transform: Transform::from_translation(Vec3::new(5.0, 0.0, 0.0)),
    };
    let far = DrawCommand {
        transform: Transform::from_translation(Vec3::new(500.0, 0.0, 0.0)),
        ..near
    };
    for command in [near, far] {
        renderer.queue_draw(command);
        renderer.queue_draw_to(overlay, command).unwrap();
    }
    renderer.render_frame().unwrap();
    // La caméra large de la principale voit les deux ; l'overlay
    // (cube NDC décalé, x ∈ [4, 6]) ne garde que le proche —
    // chaque passe cull avec SA vue, jamais celle d'une autre.
    let report = renderer.frame_report();
    assert_eq!(report.passes[0].draws, 2);
    assert_eq!(report.passes[0].culled, 0);
    assert_eq!(report.passes[1].draws, 1);
    assert_eq!(report.passes[1].culled, 1);
    let lines = render_lines(&journal);
    assert!(lines[1].contains("m=[5, 0, 0]"));
    assert!(!lines[1].contains("m=[500, 0, 0]"));
}

#[test]
fn checkpoint_culling_v1_a_stress_scene_pays_only_for_the_visible() {
    // LE checkpoint Culling V1 : mille et un objets dont ~900 hors
    // champ — la passe ne paie QUE les visibles (résolution,
    // instances, soumissions), l'ombre garde SES casters (celui
    // derrière la caméra projette encore), et deux frames rendent
    // EXACTEMENT les mêmes comptes sans pipeline de plus.
    let (mut renderer, journal) = mock_renderer();
    renderer
        .set_directional_shadow(&DirectionalShadowDescriptor::new(ShadowVolume::new(
            Vec3::new(50.0, 0.0, 0.0),
            Vec3::new(60.0, 20.0, 60.0),
        )))
        .unwrap();
    let crowd = lit_caster(&mut renderer, "crowd");
    let queue_scene = |renderer: &mut Renderer| {
        renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 0.9));
        for index in 0..1000u32 {
            let x = if index < 100 {
                index as f32
            } else {
                5000.0 + index as f32
            };
            renderer.queue_draw(DrawCommand {
                transform: Transform::from_translation(Vec3::new(x, 0.0, 0.0)),
                ..crowd
            });
        }
        // Le 1001e : DERRIÈRE la caméra (z > 0), dans le volume de
        // lumière (le z monde vit dans l'extent Y du volume — la
        // lumière est verticale) — son ombre reste au sol.
        renderer.queue_draw(DrawCommand {
            transform: Transform::from_translation(Vec3::new(50.0, 0.0, 15.0)),
            ..crowd
        });
    };

    // FRAME 1 : 1001 objets logiques → 100 résolus, UNE soumission ;
    // l'ombre en résout 101 (les visibles + celui derrière).
    queue_scene(&mut renderer);
    renderer.render_frame().unwrap();
    let report = renderer.frame_report();
    assert_eq!(report.passes[0].draws, 100);
    assert_eq!(report.passes[0].draw_calls, 1);
    assert_eq!(report.passes[0].culled, 901);
    assert_eq!(
        report.shadow,
        Some(ShadowReport {
            draws: 101,
            draw_calls: 1,
            culled: 900,
            resolution: 2048
        })
    );
    let render = render_lines(&journal).pop().unwrap();
    assert_eq!(render.matches("m=[").count(), 1);
    assert!(render.contains("inst=100"));
    assert!(shadow_lines(&journal).pop().unwrap().contains("inst=101"));

    // FRAME 2 : mêmes soumissions — mêmes comptes exacts, AUCUN
    // pipeline de plus.
    let pipelines_before = create_pipeline_lines(&journal).len();
    renderer.clear_draws();
    queue_scene(&mut renderer);
    renderer.render_frame().unwrap();
    assert_eq!(create_pipeline_lines(&journal).len(), pipelines_before);
    assert_eq!(renderer.frame_report().passes[0].culled, 901);
    assert_eq!(renderer.frame_report().shadow.unwrap().draws, 101);
}

#[test]
fn render_to_target_culls_with_its_own_camera() {
    let (mut renderer, journal) = mock_renderer();
    let material = plain_material(&mut renderer, "p");
    let mesh = renderer.create_mesh("cube", &cube()).unwrap();
    let target = small_target(&mut renderer, "viewport");
    let inside = DrawCommand {
        mesh,
        material,
        transform: Transform::IDENTITY,
    };
    let outside = DrawCommand {
        transform: Transform::from_translation(Vec3::new(5000.0, 0.0, 0.0)),
        ..inside
    };
    // Le rendu immédiat cull avec la VP QU'ON lui donne (le cube
    // NDC de l'identité) — jamais la caméra principale.
    renderer
        .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[inside, outside])
        .unwrap();
    let line = render_lines(&journal).pop().unwrap();
    assert_eq!(line.matches("m=[").count(), 1);
    assert!(line.contains("m=[0, 0, 0]"));
    assert!(!line.contains("5000"));
}
