//! Le debug rendering : routage frame/retenu, expiration, toggles,
//! batches Scene et Overlay, permutations par format et checkpoint V1.

use super::*;

#[test]
fn debug_draws_route_by_duration_and_expire() {
    let (mut renderer, _journal) = mock_renderer();
    renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X));
    renderer.queue_debug(DebugDraw::marker(Vec3::ZERO, 0.5).with_duration(2.0));
    assert_eq!(
        renderer.debug_stats(),
        DebugStats {
            frame: 1,
            retained: 1
        }
    );
    // `clear_draws` vide la frame, JAMAIS les retenues.
    renderer.clear_draws();
    assert_eq!(
        renderer.debug_stats(),
        DebugStats {
            frame: 0,
            retained: 1
        }
    );
    // Le temps décompte ; un delta invalide est ignoré.
    renderer.advance_debug_time(1.0);
    renderer.advance_debug_time(f32::NAN);
    renderer.advance_debug_time(-5.0);
    assert_eq!(renderer.debug_stats().retained, 1);
    renderer.advance_debug_time(1.0);
    assert_eq!(
        renderer.debug_stats(),
        DebugStats {
            frame: 0,
            retained: 0
        }
    );
}

#[test]
fn invalid_debug_draws_are_dropped_at_submit() {
    let (mut renderer, _journal) = mock_renderer();
    renderer.queue_debug(DebugDraw::line(Vec3::new(f32::NAN, 0.0, 0.0), Vec3::X));
    renderer.queue_debug(DebugDraw::sphere(Vec3::ZERO, 0.0));
    renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X).with_duration(-1.0));
    renderer.queue_debug(DebugDraw::point(Vec3::ZERO, 0.1).for_pass(PassHandle(42)));
    assert_eq!(renderer.debug_stats(), DebugStats::default());
    renderer.queue_debug(DebugDraw::point(Vec3::ZERO, 0.1));
    assert_eq!(renderer.debug_stats().frame, 1);
}

#[test]
fn debug_toggles_flip_the_rendering_state() {
    let (mut renderer, _journal) = mock_renderer();
    assert!(renderer.debug_enabled());
    renderer.set_debug_enabled(false);
    assert!(!renderer.debug_enabled());
    assert!(renderer.debug_category_enabled("physics"));
    renderer.set_debug_category_enabled("physics", false);
    assert!(!renderer.debug_category_enabled("physics"));
    assert!(renderer.debug_category_enabled(DEFAULT_DEBUG_CATEGORY));
    renderer.set_debug_category_enabled("physics", true);
    assert!(renderer.debug_category_enabled("physics"));
}

#[test]
fn debug_is_injected_after_the_transparents() {
    let (mut renderer, journal) = mock_renderer();
    let glass = renderer
        .create_material(
            &MaterialDescriptor::new("glass", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
    let mesh = lit_quad_mesh(&mut renderer, "pane");
    renderer.queue_draw(DrawCommand {
        mesh,
        material: glass,
        transform: Transform::from_translation(Vec3::new(0.0, 0.0, -1.0)),
    });
    renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X));
    renderer.render_frame().unwrap();
    // Le batch debug voyage APRÈS les draws de la passe (le slot
    // réservé) — le suffixe `dbg` du journal le porte, et sa
    // permutation est lignes + blend + LessEqual sans écriture.
    let line = render_lines(&journal).pop().unwrap();
    assert!(line.contains(" dbg=[v=2 p="));
    assert!(create_pipeline_lines(&journal).iter().any(|line| {
        line.starts_with("create_pipeline chaos.debug ")
            && line.contains(" blend=alpha")
            && line.contains(" depth=less_equal")
            && line.contains(" topology=lines")
    }));
    let report = &renderer.frame_report().passes[0];
    assert_eq!(report.breakdown.transparent, 1);
    assert_eq!(report.breakdown.injected, 1);
    assert_eq!(report.draws, 2);
    assert_eq!(report.draw_calls, 2);
}

#[test]
fn the_overlay_batch_comes_last_with_its_own_permutation() {
    let (mut renderer, journal) = mock_renderer();
    renderer.queue_debug(DebugDraw::point(Vec3::ZERO, 0.5).overlay());
    renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X));
    renderer.render_frame().unwrap();
    // Deux batches : Scene D'ABORD, l'overlay en DERNIER (il
    // dessine par-dessus) — chacun sa permutation, `.overlay` en
    // profondeur Always.
    let line = render_lines(&journal).pop().unwrap();
    let scene_at = line.find(" dbg=[v=2 p=").unwrap();
    let overlay_at = line.find(", v=6 p=").unwrap();
    assert!(scene_at < overlay_at);
    assert!(create_pipeline_lines(&journal).iter().any(|line| {
        line.starts_with("create_pipeline chaos.debug.overlay ")
            && line.contains(" depth=always")
            && line.contains(" topology=lines")
    }));
    let report = &renderer.frame_report().passes[0];
    assert_eq!(report.breakdown.injected, 2);
    assert_eq!(report.draw_calls, 2);
}

#[test]
fn debug_routes_to_its_target_pass_with_its_format() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let mirror = renderer
        .add_pass(
            &RenderPassDescriptor::new("mirror", RenderDestination::Target(target)).with_order(-1),
        )
        .unwrap();
    renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X));
    renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::Y).for_pass(mirror));
    renderer.render_frame().unwrap();
    // Chaque passe reçoit SON debug — et la passe cible résout la
    // permutation de SON format (le descripteur, pas que le label).
    let lines = render_lines(&journal);
    assert!(lines[0].contains("pass=mirror"));
    assert!(lines[0].contains(" dbg=[v=2 p="));
    assert!(lines[1].contains(" dbg=[v=2 p="));
    assert!(create_pipeline_lines(&journal).iter().any(|line| {
        line.starts_with("create_pipeline chaos.debug.Rgba8UnormSrgb ")
            && line.contains(" target=Rgba8UnormSrgb")
    }));
    let report = renderer.frame_report();
    assert_eq!(report.passes[0].breakdown.injected, 1);
    assert_eq!(report.passes[1].breakdown.injected, 1);
}

#[test]
fn disabled_debug_leaves_the_journal_clean() {
    let (mut renderer, journal) = mock_renderer();
    renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X));
    renderer.queue_debug(DebugDraw::sphere(Vec3::ZERO, 1.0).with_category("bounds"));
    // Le toggle GLOBAL coupe tout : la ligne render est EXACTEMENT
    // la ligne historique — zéro delta de journal.
    renderer.set_debug_enabled(false);
    renderer.render_frame().unwrap();
    assert_eq!(
        render_lines(&journal).pop().unwrap(),
        "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[]"
    );
    assert_eq!(renderer.frame_report().passes[0].breakdown.injected, 0);
    // La CATÉGORIE filtre au rendu : `bounds` coupée, la ligne
    // reste — un seul batch.
    renderer.set_debug_enabled(true);
    renderer.set_debug_category_enabled("bounds", false);
    renderer.render_frame().unwrap();
    let line = render_lines(&journal).pop().unwrap();
    assert!(line.contains(" dbg=[v=2 p="));
    assert!(!line.contains(", v="));
    // Réveillée, la sphère revient (144 sommets + 2 de la ligne).
    renderer.set_debug_category_enabled("bounds", true);
    renderer.render_frame().unwrap();
    assert!(
        render_lines(&journal)
            .pop()
            .unwrap()
            .contains(" dbg=[v=146 p=")
    );
}

#[test]
fn retained_debug_survives_clear_draws_and_expires_on_screen() {
    let (mut renderer, journal) = mock_renderer();
    renderer.queue_debug(DebugDraw::marker(Vec3::ZERO, 0.5).with_duration(5.0));
    renderer.render_frame().unwrap();
    assert!(render_lines(&journal).pop().unwrap().contains(" dbg=["));
    // `clear_draws` (la frame de simulation suivante) : la retenue
    // est TOUJOURS dessinée.
    renderer.clear_draws();
    renderer.render_frame().unwrap();
    assert!(render_lines(&journal).pop().unwrap().contains(" dbg=["));
    // Le temps l'expire : plus aucun batch, la ligne redevient
    // historique.
    renderer.advance_debug_time(6.0);
    renderer.render_frame().unwrap();
    assert!(!render_lines(&journal).pop().unwrap().contains(" dbg=["));
}

#[test]
fn checkpoint_debug_v1_the_visual_language_lives_and_expires() {
    // LE checkpoint Debug Rendering V1 : toutes les formes sous les
    // deux modes de profondeur, une scène régulière à côté, une
    // retenue qui survit aux frames et expire par le temps, les
    // catégories togglées à chaud, les comptes exacts — et AUCUN
    // pipeline de plus une fois les permutations chaudes.
    let (mut renderer, journal) = mock_renderer();
    let material = plain_material(&mut renderer, "prop");
    let mesh = renderer.create_mesh("cube", &cube()).unwrap();
    let queue_scene = |renderer: &mut Renderer| {
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        renderer.queue_debug(DebugDraw::grid(Vec3::ZERO, 10.0, 1.0));
        renderer.queue_debug(DebugDraw::axes(Mat4::IDENTITY, 2.0).overlay());
        renderer.queue_debug(DebugDraw::aabb(
            Aabb::from_points([Vec3::ZERO, Vec3::ONE]).unwrap(),
        ));
        renderer.queue_debug(DebugDraw::sphere(Vec3::ZERO, 1.0).with_category("bounds"));
        renderer.queue_debug(DebugDraw::frustum(projection::orthographic(
            -1.0, 1.0, -1.0, 1.0, 0.0, 10.0,
        )));
        renderer.queue_debug(DebugDraw::ray(Vec3::ZERO, Vec3::X));
        renderer.queue_debug(DebugDraw::arrow(Vec3::ZERO, Vec3::Y));
        renderer.queue_debug(DebugDraw::point(Vec3::ZERO, 0.2));
        renderer.queue_debug(DebugDraw::light(
            &Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0),
            Vec3::new(0.0, 5.0, 0.0),
        ));
        renderer.queue_debug(DebugDraw::light(
            &Light::point(Vec3::X, Color::WHITE, 1.0, 2.0),
            Vec3::ZERO,
        ));
        renderer.queue_debug(DebugDraw::light(
            &Light::spot(Vec3::Y, Vec3::NEG_Y, Color::WHITE, 1.0, 3.0, 0.3, 0.5),
            Vec3::ZERO,
        ));
    };

    // FRAME 1 : 11 primitives immédiates + 1 retenue (3 s) — DEUX
    // batches (Scene puis Overlay), 12 injectées, 3 soumissions (le
    // cube + les deux batches).
    queue_scene(&mut renderer);
    renderer.queue_debug(
        DebugDraw::marker(Vec3::ZERO, 0.5)
            .with_duration(3.0)
            .with_category("markers"),
    );
    renderer.render_frame().unwrap();
    let report = &renderer.frame_report().passes[0];
    assert_eq!(report.breakdown.injected, 12);
    assert_eq!(report.draws, 13);
    assert_eq!(report.draw_calls, 3);
    let line = render_lines(&journal).pop().unwrap();
    assert_eq!(line.matches("v=").count(), 2);
    let pipelines_after_first = create_pipeline_lines(&journal).len();

    // FRAME 2 : la simulation avance (clear + 1 s) — seule la
    // RETENUE survit, sur un batch Scene, sans pipeline de plus.
    renderer.clear_draws();
    renderer.advance_debug_time(1.0);
    renderer.render_frame().unwrap();
    let report = &renderer.frame_report().passes[0];
    assert_eq!(report.breakdown.injected, 1);
    assert_eq!(report.draws, 1);
    assert_eq!(report.draw_calls, 1);
    assert_eq!(create_pipeline_lines(&journal).len(), pipelines_after_first);

    // FRAME 3 : sa catégorie coupée — plus rien à l'écran, la
    // retenue continue d'expirer en coulisses.
    renderer.set_debug_category_enabled("markers", false);
    renderer.advance_debug_time(1.0);
    renderer.render_frame().unwrap();
    assert_eq!(renderer.frame_report().passes[0].breakdown.injected, 0);
    assert_eq!(renderer.debug_stats().retained, 1);

    // FRAME 4 : réveillée puis EXPIRÉE — le journal redevient
    // exactement historique.
    renderer.set_debug_category_enabled("markers", true);
    renderer.advance_debug_time(1.5);
    renderer.render_frame().unwrap();
    assert_eq!(renderer.debug_stats().retained, 0);
    assert_eq!(
        render_lines(&journal).pop().unwrap(),
        "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[]"
    );
}

#[test]
fn render_to_target_draws_no_debug() {
    let (mut renderer, journal) = mock_renderer();
    let material = plain_material(&mut renderer, "p");
    let mesh = renderer.create_mesh("cube", &cube()).unwrap();
    let target = small_target(&mut renderer, "viewport");
    renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X));
    renderer
        .render_to_target(
            target,
            Color::BLACK,
            Mat4::IDENTITY,
            &[DrawCommand {
                mesh,
                material,
                transform: Transform::IDENTITY,
            }],
        )
        .unwrap();
    // Le chemin immédiat ne dessine PAS de debug — la règle de la
    // passe d'ombre.
    assert!(!render_lines(&journal).pop().unwrap().contains(" dbg=["));
}
