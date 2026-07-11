//! Le plan de frame et le registre des passes : clear color, resize, files
//! de draw et leur résolution, ordonnancement, dépendances et caméras.

use super::*;

#[test]
fn frame_plan_carries_current_clear_color() {
    let (mut renderer, journal) = mock_renderer();
    renderer.render_frame().unwrap();
    assert_eq!(
        journal.entries(),
        vec!["render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[]"]
    );
}

#[test]
fn set_clear_color_changes_the_plan() {
    let (mut renderer, journal) = mock_renderer();
    renderer.set_clear_color(Color::rgb(1.0, 0.5, 0.25));
    renderer.render_frame().unwrap();
    assert_eq!(
        journal.entries(),
        vec!["render r=1 g=0.5 b=0.25 a=1 vp=[0, 0, 0] draws=[]"]
    );
    assert_eq!(renderer.clear_color(), Color::rgb(1.0, 0.5, 0.25));
}

#[test]
fn resize_is_forwarded_to_backend() {
    let (mut renderer, journal) = mock_renderer();
    renderer.resize(1920, 1080);
    assert_eq!(journal.entries(), vec!["resize 1920x1080"]);
    assert_eq!(renderer.surface_size(), (1920, 1080));
    renderer.resize(0, 0);
    assert_eq!(renderer.surface_size(), (1920, 1080));
}

#[test]
fn draw_count_reports_the_submitted_frame() {
    let (mut renderer, _journal) = mock_renderer();
    let material = plain_material(&mut renderer, "p");
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    assert_eq!(renderer.draw_count(), 0);
    renderer.queue_draw(DrawCommand {
        mesh,
        material,
        transform: Transform::IDENTITY,
    });
    renderer.queue_draw(DrawCommand {
        mesh,
        material,
        transform: Transform::IDENTITY,
    });
    assert_eq!(renderer.draw_count(), 2);
    renderer.clear_draws();
    assert_eq!(renderer.draw_count(), 0);
}

#[test]
fn stale_material_draw_is_dropped_from_the_plan() {
    let (mut renderer, journal) = mock_renderer();
    let material = plain_material(&mut renderer, "p");
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    renderer.queue_draw(DrawCommand {
        mesh,
        material,
        transform: Transform::IDENTITY,
    });
    renderer.destroy_material(material).unwrap();
    renderer.render_frame().unwrap();
    assert!(journal.entries().contains(&String::from(
        "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[]"
    )));
}

#[test]
fn create_mesh_uploads_the_right_buffers() {
    let (mut renderer, journal) = mock_renderer();
    let first = renderer.create_mesh("tri", &triangle()).unwrap();
    let second = renderer.create_mesh("quad", &quad()).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        journal.entries(),
        vec![
            "create_buffer tri kind=Vertex bytes=72",
            "create_buffer quad kind=Vertex bytes=96",
            "create_buffer quad.indices kind=Index bytes=12"
        ]
    );
}

#[test]
fn create_textured_mesh_uploads_the_uv_vertices() {
    let (mut renderer, journal) = mock_renderer();
    let quad = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 4.0);
    renderer.create_textured_mesh("floor", &quad).unwrap();
    assert_eq!(
        journal.entries(),
        vec![
            "create_buffer floor kind=Vertex bytes=80",
            "create_buffer floor.indices kind=Index bytes=12"
        ]
    );
}

#[test]
fn mesh_draws_resolve_into_the_plan_then_reset() {
    let (mut renderer, journal) = mock_renderer();
    let material = plain_material(&mut renderer, "p");
    let tri = renderer.create_mesh("tri", &triangle()).unwrap();
    let quad = renderer.create_mesh("quad", &quad()).unwrap();
    renderer.queue_draw(DrawCommand {
        mesh: tri,
        material,
        transform: Transform::IDENTITY,
    });
    renderer.queue_draw(DrawCommand {
        mesh: quad,
        material,
        transform: Transform::from_translation(Vec3::new(5.0, 0.0, 0.0)),
    });
    renderer.render_frame().unwrap();
    renderer.render_frame().unwrap();
    renderer.clear_draws();
    renderer.render_frame().unwrap();
    let entries = journal.entries();
    let full_plan = "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[(0, Some(0), None, 3, b=Some(0), m=[0, 0, 0]), (0, Some(1), Some(2), 6, b=Some(0), m=[5, 0, 0])]";
    assert_eq!(entries[entries.len() - 3], full_plan);
    assert_eq!(entries[entries.len() - 2], full_plan);
    assert_eq!(
        entries[entries.len() - 1],
        "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[]"
    );
}

#[test]
fn interleaved_materials_are_grouped_in_the_plan() {
    let (mut renderer, journal) = mock_renderer();
    let first = plain_material(&mut renderer, "a");
    let second = plain_material(&mut renderer, "b");
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    let submissions = [(second, 1.0), (first, 2.0), (second, 3.0), (first, 4.0)];
    for (material, x) in submissions {
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::from_translation(Vec3::new(x, 0.0, 0.0)),
        });
    }
    renderer.render_frame().unwrap();
    let entries = journal.entries();
    // Les deux materials partagent le MÊME modèle et le même état :
    // les runs de chacun (mesh partagé) FUSIONNENT en un draw
    // instancié — la permutation instanciée (pipeline 1, lazy à la
    // première frame) est DÉDUPLIQUÉE entre les deux, seuls les
    // bindings diffèrent ; la matrice du draw est celle de la
    // PREMIÈRE instance, l'ordre de soumission tient dans le run.
    assert_eq!(
        entries[entries.len() - 1],
        "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[\
         (1, Some(0), None, 3, b=Some(0), m=[2, 0, 0], inst=2), \
         (1, Some(0), None, 3, b=Some(1), m=[1, 0, 0], inst=2)]"
    );
    assert_eq!(
        create_pipeline_lines(&journal)
            .iter()
            .filter(|line| line.contains(" instanced"))
            .count(),
        1
    );
}

#[test]
fn shared_mesh_draws_with_distinct_transforms() {
    let (mut renderer, journal) = mock_renderer();
    let material = plain_material(&mut renderer, "p");
    let mesh = renderer.create_mesh("cube", &cube()).unwrap();
    for x in [-2.0, 0.0, 2.0] {
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::from_translation(Vec3::new(x, 0.0, 0.0)),
        });
    }
    renderer.render_frame().unwrap();
    let entries = journal.entries();
    // Le motif « un mesh, N draws » est devenu LA forme instanciée :
    // UN draw, trois instances, la matrice de la première en tête.
    assert_eq!(
        entries[entries.len() - 1],
        "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[\
         (1, Some(0), Some(1), 36, b=Some(0), m=[-2, 0, 0], inst=3)]"
    );
}

#[test]
fn many_draws_reach_the_plan_in_submission_order() {
    // Le chemin CLASSIQUE : seize meshes distincts (aucun run à
    // fusionner) — chaque draw garde son slot, l'ordre déterministe
    // du tri (material, mesh) suit l'ordre de création.
    let (mut renderer, journal) = mock_renderer();
    let material = plain_material(&mut renderer, "p");
    for index in 0u8..16 {
        let mesh = renderer
            .create_mesh(&format!("cube.{index}"), &cube())
            .unwrap();
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::from_translation(Vec3::new(f32::from(index), 0.0, 0.0)),
        });
    }
    renderer.render_frame().unwrap();
    let entries = journal.entries();
    let plan = entries[entries.len() - 1].clone();
    assert_eq!(plan.matches("m=[").count(), 16);
    let positions: Vec<usize> = (0u8..16)
        .map(|index| {
            plan.find(&format!("m=[{}, 0, 0]", f32::from(index)))
                .unwrap()
        })
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn meshes_carry_their_local_bounds() {
    let (mut renderer, _journal) = mock_renderer();
    // Le cube unitaire à l'origine : bounds exacts ±0.5.
    let cube = renderer.create_mesh("cube", &cube()).unwrap();
    let bounds = renderer.mesh_bounds(cube).unwrap().unwrap();
    assert_eq!(bounds.min, Vec3::splat(-0.5));
    assert_eq!(bounds.max, Vec3::splat(0.5));
    // Une géométrie VIDE n'a pas de bounds — jamais cullée.
    let empty = renderer
        .create_mesh(
            "empty",
            &Geometry {
                vertices: Vec::new(),
                indices: Vec::new(),
            },
        )
        .unwrap();
    assert!(renderer.mesh_bounds(empty).unwrap().is_none());
    // Une position non finie REFUSE les bounds (warn) — jamais cullé.
    let broken = renderer
        .create_mesh(
            "broken",
            &Geometry {
                vertices: vec![ColorVertex {
                    position: [f32::NAN, 0.0, 0.0],
                    color: [1.0, 1.0, 1.0],
                }],
                indices: Vec::new(),
            },
        )
        .unwrap();
    assert!(renderer.mesh_bounds(broken).unwrap().is_none());
    // L'inspection refuse un handle périmé.
    renderer.destroy_mesh(broken).unwrap();
    assert!(renderer.mesh_bounds(broken).is_err());
}

#[test]
fn destroy_mesh_destroys_its_buffers() {
    let (mut renderer, journal) = mock_renderer();
    let mesh = renderer.create_mesh("quad", &quad()).unwrap();
    renderer.destroy_mesh(mesh).unwrap();
    renderer.render_frame().unwrap();
    let entries = journal.entries();
    assert!(entries.contains(&String::from("destroy_buffer index=0")));
    assert!(entries.contains(&String::from("destroy_buffer index=1")));
    let error = renderer.destroy_mesh(mesh).unwrap_err();
    assert!(error.to_string().contains("stale"));
}

#[test]
fn stale_mesh_draw_is_dropped_from_the_plan() {
    let (mut renderer, journal) = mock_renderer();
    let material = plain_material(&mut renderer, "p");
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    renderer.queue_draw(DrawCommand {
        mesh,
        material,
        transform: Transform::IDENTITY,
    });
    renderer.destroy_mesh(mesh).unwrap();
    renderer.render_frame().unwrap();
    assert!(journal.entries().contains(&String::from(
        "render r=0 g=0 b=0 a=1 vp=[0, 0, 0] draws=[]"
    )));
}

#[test]
fn view_projection_travels_in_the_plan() {
    let (mut renderer, journal) = mock_renderer();
    renderer.set_view_projection(Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)));
    renderer.render_frame().unwrap();
    assert_eq!(
        journal.entries(),
        vec!["render r=0 g=0 b=0 a=1 vp=[1, 2, 3] draws=[]"]
    );
}

#[test]
fn the_order_drives_the_schedule_not_the_registration() {
    let (mut renderer, journal) = mock_renderer();
    renderer
        .add_pass(&surface_pass("after").with_order(5))
        .unwrap();
    renderer
        .add_pass(&surface_pass("before").with_order(-5))
        .unwrap();
    renderer
        .add_pass(&surface_pass("tied").with_order(5))
        .unwrap();
    renderer.render_frame().unwrap();
    renderer.render_frame().unwrap();
    let lines = render_lines(&journal);
    // Deux frames — le même ordre exact : before, main, after, tied
    // (l'égalité d'ordre départagée par l'enregistrement).
    assert_eq!(lines.len(), 8);
    for frame in lines.chunks(4) {
        assert!(frame[0].ends_with(" pass=before"));
        assert!(!frame[1].contains(" pass="));
        assert!(frame[2].ends_with(" pass=after"));
        assert!(frame[3].ends_with(" pass=tied"));
    }
}

#[test]
fn pass_labels_are_validated() {
    let (mut renderer, _journal) = mock_renderer();
    let empty = renderer.add_pass(&surface_pass("")).unwrap_err();
    assert!(empty.to_string().contains("cannot be empty"));
    let reserved = renderer
        .add_pass(&surface_pass("chaos.shadow"))
        .unwrap_err();
    assert!(reserved.to_string().contains("reserved for engine passes"));
    renderer.add_pass(&surface_pass("overlay")).unwrap();
    let duplicate = renderer.add_pass(&surface_pass("overlay")).unwrap_err();
    assert!(duplicate.to_string().contains("'overlay' already exists"));
}

#[test]
fn invalid_dependencies_are_refused_by_name() {
    let (mut renderer, _journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");

    let feedback = renderer
        .add_pass(
            &RenderPassDescriptor::new("loop", RenderDestination::Target(target))
                .with_reads(&[target]),
        )
        .unwrap_err();
    assert!(feedback.to_string().contains("feedback loop"));

    // Lectrice à l'ordre -1, écrivaine à l'ordre 0 : l'écrivaine
    // arriverait APRÈS la lecture — refusée en nommant tout le monde.
    renderer
        .add_pass(&surface_pass("reader").with_reads(&[target]).with_order(-1))
        .unwrap();
    let writer = renderer
        .add_pass(&RenderPassDescriptor::new(
            "writer",
            RenderDestination::Target(target),
        ))
        .unwrap_err();
    let message = writer.to_string();
    assert!(message.contains("'writer' writes 'viewport' after pass 'reader' reads it"));
    assert!(message.contains("schedule it earlier"));

    // La même écrivaine AVANT la lectrice est la forme légale.
    renderer
        .add_pass(
            &RenderPassDescriptor::new("writer", RenderDestination::Target(target)).with_order(-2),
        )
        .unwrap();
}

#[test]
fn a_read_without_a_writer_is_legal() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    renderer
        .add_pass(&surface_pass("reader").with_reads(&[target]))
        .unwrap();
    renderer.render_frame().unwrap();
    // Personne n'écrit la cible cette frame : contenu d'une frame
    // précédente, la passe s'exécute quand même.
    assert_eq!(render_lines(&journal).len(), 2);
}

#[test]
fn update_pass_revalidates_the_whole_schedule() {
    let (mut renderer, _journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let writer = renderer
        .add_pass(
            &RenderPassDescriptor::new("writer", RenderDestination::Target(target)).with_order(-2),
        )
        .unwrap();
    renderer
        .add_pass(&surface_pass("reader").with_reads(&[target]).with_order(-1))
        .unwrap();

    // Repousser l'écrivaine APRÈS la lectrice casse l'invariant
    // entre deux passes que l'update ne touche pas directement.
    let pushed =
        RenderPassDescriptor::new("writer", RenderDestination::Target(target)).with_order(3);
    let refused = renderer.update_pass(writer, &pushed).unwrap_err();
    assert!(refused.to_string().contains("schedule it earlier"));

    // Refus = état intact : l'ordre d'origine tient toujours.
    let kept =
        RenderPassDescriptor::new("writer", RenderDestination::Target(target)).with_order(-3);
    renderer.update_pass(writer, &kept).unwrap();
}

#[test]
fn the_main_pass_destination_and_label_are_protected() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let main = renderer.main_pass();

    let moved = renderer
        .update_pass(
            main,
            &RenderPassDescriptor::new("chaos.main", RenderDestination::Target(target)),
        )
        .unwrap_err();
    assert!(moved.to_string().contains("destination cannot change"));

    let renamed = renderer
        .update_pass(main, &surface_pass("scene"))
        .unwrap_err();
    assert!(renamed.to_string().contains("label cannot change"));

    // load / caméra / ordre restent libres sur main.
    renderer
        .update_pass(
            main,
            &RenderPassDescriptor::new("chaos.main", RenderDestination::Surface)
                .with_load(PassLoad::Keep)
                .with_order(10),
        )
        .unwrap();
    renderer.render_frame().unwrap();
    assert!(render_lines(&journal)[0].ends_with(" load=keep"));
}

#[test]
fn a_disabled_pass_is_skipped_cleanly() {
    let (mut renderer, journal) = mock_renderer();
    let overlay = renderer.add_pass(&surface_pass("overlay")).unwrap();
    let material = plain_material(&mut renderer, "p");
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    let command = DrawCommand {
        mesh,
        material,
        transform: Transform::IDENTITY,
    };

    renderer.set_pass_enabled(overlay, false).unwrap();
    renderer.queue_draw_to(overlay, command).unwrap();
    renderer.render_frame().unwrap();
    assert_eq!(render_lines(&journal).len(), 1);
    let report = renderer.frame_report();
    assert_eq!(report.passes.len(), 2);
    assert_eq!(report.passes[1].label, "overlay");
    assert_eq!(report.passes[1].outcome, PassOutcome::Disabled);

    renderer.set_pass_enabled(overlay, true).unwrap();
    renderer.render_frame().unwrap();
    assert_eq!(render_lines(&journal).len(), 3);
}

#[test]
fn each_pass_owns_its_queue_and_the_count_sums_them() {
    let (mut renderer, journal) = mock_renderer();
    let overlay = renderer.add_pass(&surface_pass("overlay")).unwrap();
    let material = plain_material(&mut renderer, "p");
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    let command = DrawCommand {
        mesh,
        material,
        transform: Transform::IDENTITY,
    };

    renderer.queue_draw(command);
    renderer.queue_draw_to(overlay, command).unwrap();
    renderer.queue_draw_to(overlay, command).unwrap();
    assert_eq!(renderer.draw_count(), 3);

    renderer.render_frame().unwrap();
    let lines = render_lines(&journal);
    assert_eq!(lines[0].matches("m=[").count(), 1);
    // Les deux draws identiques de l'overlay FUSIONNENT — un draw
    // instancié, deux instances : le compte logique ne ment pas.
    assert_eq!(lines[1].matches("m=[").count(), 1);
    assert!(lines[1].contains("inst=2"));
    assert_eq!(renderer.frame_report().passes[1].draws, 2);
    assert_eq!(renderer.frame_report().passes[1].draw_calls, 1);

    renderer.clear_draws();
    assert_eq!(renderer.draw_count(), 0);

    let unknown = renderer.queue_draw_to(PassHandle(42), command).unwrap_err();
    assert!(unknown.to_string().contains("unknown"));
}

#[test]
fn each_pass_travels_with_its_own_camera() {
    let (mut renderer, journal) = mock_renderer();
    let overlay = renderer.add_pass(&surface_pass("overlay")).unwrap();
    renderer.set_view_projection(Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0)));
    renderer
        .set_pass_camera(overlay, Mat4::from_translation(Vec3::new(0.0, 2.0, 0.0)))
        .unwrap();
    renderer.render_frame().unwrap();
    let lines = render_lines(&journal);
    assert!(lines[0].contains("vp=[1, 0, 0]"));
    assert!(lines[1].contains("vp=[0, 2, 0]"));
}

#[test]
fn a_stale_destination_disables_the_pass_until_updated() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let mirror = renderer
        .add_pass(
            &RenderPassDescriptor::new("mirror", RenderDestination::Target(target)).with_order(-1),
        )
        .unwrap();

    let fresh = renderer.resize_render_target(target, 8, 8).unwrap();
    renderer.render_frame().unwrap();
    let report = renderer.frame_report();
    assert_eq!(report.passes[0].label, "mirror");
    assert_eq!(report.passes[0].outcome, PassOutcome::StaleTarget);
    assert_eq!(render_lines(&journal).len(), 1);

    // Auto-désactivée : la frame suivante la voit Disabled, sans
    // nouveau warn — puis update_pass la rebranche sur le handle frais.
    renderer.render_frame().unwrap();
    assert_eq!(
        renderer.frame_report().passes[0].outcome,
        PassOutcome::Disabled
    );
    renderer
        .update_pass(
            mirror,
            &RenderPassDescriptor::new("mirror", RenderDestination::Target(fresh)).with_order(-1),
        )
        .unwrap();
    renderer.render_frame().unwrap();
    assert_eq!(
        renderer.frame_report().passes[0].outcome,
        PassOutcome::Executed
    );
    assert!(
        render_lines(&journal)
            .last()
            .is_some_and(|line| !line.contains("pass="))
    );
}

#[test]
fn an_undeclared_feedback_draw_is_dropped() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let color = renderer.render_target_color(target).unwrap();
    let looping = renderer
        .create_material(
            &MaterialDescriptor::new("looping", MaterialModel::Unlit).with_texture(color),
        )
        .unwrap();
    let sane = plain_material(&mut renderer, "sane");
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    let screen_quad = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0);
    let textured_mesh = renderer.create_textured_mesh("quad", &screen_quad).unwrap();

    let mirror = renderer
        .add_pass(
            &RenderPassDescriptor::new("mirror", RenderDestination::Target(target)).with_order(-1),
        )
        .unwrap();
    renderer
        .queue_draw_to(
            mirror,
            DrawCommand {
                mesh: textured_mesh,
                material: looping,
                transform: Transform::IDENTITY,
            },
        )
        .unwrap();
    renderer
        .queue_draw_to(
            mirror,
            DrawCommand {
                mesh,
                material: sane,
                transform: Transform::IDENTITY,
            },
        )
        .unwrap();
    renderer.render_frame().unwrap();
    // Le draw qui échantillonne la destination est écarté, l'autre passe.
    let mirror_line = render_lines(&journal)
        .into_iter()
        .find(|line| line.contains("pass=mirror"))
        .unwrap();
    assert_eq!(mirror_line.matches("m=[").count(), 1);
    assert_eq!(renderer.frame_report().passes[0].draws, 1);
}

#[test]
fn an_empty_plan_still_flushes_the_retirement() {
    let (mut renderer, journal) = mock_renderer();
    renderer
        .set_pass_enabled(renderer.main_pass(), false)
        .unwrap();
    let material = plain_material(&mut renderer, "p");
    renderer.destroy_material(material).unwrap();
    assert_eq!(renderer.resource_stats().retired, 1);

    let outcome = renderer.render_frame().unwrap();
    assert_eq!(outcome, FrameOutcome::Rendered);
    assert_eq!(renderer.resource_stats().retired, 0);
    assert!(render_lines(&journal).is_empty());
    assert_eq!(
        renderer.frame_report().passes[0].outcome,
        PassOutcome::Disabled
    );
}

#[test]
fn target_passes_survive_a_skipped_surface() {
    let (mut renderer, journal) =
        mock_renderer_with(FrameOutcome::Skipped(FrameSkipReason::ZeroArea));
    let target = small_target(&mut renderer, "viewport");
    renderer
        .add_pass(
            &RenderPassDescriptor::new("mirror", RenderDestination::Target(target)).with_order(-1),
        )
        .unwrap();
    let outcome = renderer.render_frame().unwrap();
    assert_eq!(outcome, FrameOutcome::Skipped(FrameSkipReason::ZeroArea));
    // Le mock journalise tout le plan ; la vérité du rapport vient de
    // l'inférence du renderer : cible exécutée, surface sautée.
    assert!(!render_lines(&journal).is_empty());
    let report = renderer.frame_report();
    assert_eq!(report.passes[0].outcome, PassOutcome::Executed);
    assert_eq!(report.passes[1].outcome, PassOutcome::SurfaceSkipped);
}

#[test]
fn the_report_covers_the_orchestrated_frame_only() {
    let (mut renderer, _journal) = mock_renderer();
    assert!(renderer.frame_report().passes.is_empty());

    let target = small_target(&mut renderer, "viewport");
    renderer.render_frame().unwrap();
    assert_eq!(renderer.frame_report().passes.len(), 1);
    assert_eq!(renderer.frame_report().passes[0].label, "chaos.main");

    renderer
        .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[])
        .unwrap();
    assert_eq!(renderer.frame_report().passes.len(), 1);
}

#[test]
fn render_to_target_is_the_offscreen_pass() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    renderer
        .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[])
        .unwrap();
    let line = render_lines(&journal).pop().unwrap();
    assert!(line.contains(" dest=target0 pass=chaos.offscreen"));
}

#[test]
fn the_declared_mirror_flow_is_the_checkpoint() {
    let (mut renderer, journal) = mock_renderer();

    // La cible, UN material de scène partagé par les deux passes (la
    // permutation offscreen se résout seule), le material qui
    // échantillonne sa couleur — le flux réel de la démo.
    let target = small_target(&mut renderer, "viewport");
    let scene_material = plain_material(&mut renderer, "scene");
    let color = renderer.render_target_color(target).unwrap();
    let screen_material = renderer
        .create_material(
            &MaterialDescriptor::new("screen", MaterialModel::Unlit).with_texture(color),
        )
        .unwrap();
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    let screen_quad = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0);
    let screen_mesh = renderer
        .create_textured_mesh("screen", &screen_quad)
        .unwrap();

    let mirror = renderer
        .add_pass(
            &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                .with_load(PassLoad::Clear(Color::rgb(0.1, 0.0, 0.2)))
                .with_order(-10),
        )
        .unwrap();

    for _ in 0..2 {
        renderer.clear_draws();
        renderer
            .queue_draw_to(
                mirror,
                DrawCommand {
                    mesh,
                    material: scene_material,
                    transform: Transform::IDENTITY,
                },
            )
            .unwrap();
        renderer.queue_draw(DrawCommand {
            mesh,
            material: scene_material,
            transform: Transform::IDENTITY,
        });
        renderer.queue_draw(DrawCommand {
            mesh: screen_mesh,
            material: screen_material,
            transform: Transform::IDENTITY,
        });
        renderer.render_frame().unwrap();
    }

    // Deux frames, le même ordre stable : la passe miroir (cible,
    // clear violet, le MÊME material que la scène) puis la
    // principale (scène + écran) — et le rapport les nomme.
    let lines = render_lines(&journal);
    assert_eq!(lines.len(), 4);
    for frame in lines.chunks(2) {
        assert!(frame[0].starts_with("render r=0.1 g=0 b=0.2 a=1"));
        assert!(frame[0].contains(" dest=target0 pass=mirror"));
        assert!(!frame[1].contains(" pass="));
        assert_eq!(frame[1].matches("m=[").count(), 2);
    }
    let report = renderer.frame_report();
    assert_eq!(report.passes[0].label, "mirror");
    assert_eq!(report.passes[0].draws, 1);
    assert_eq!(report.passes[0].outcome, PassOutcome::Executed);
    assert_eq!(report.passes[1].label, "chaos.main");
    assert_eq!(report.passes[1].draws, 2);
    assert_eq!(report.passes[1].outcome, PassOutcome::Executed);

    // Les configurations incohérentes restent des erreurs explicites.
    assert!(renderer.add_pass(&surface_pass("mirror")).is_err());
    assert!(
        renderer
            .add_pass(
                &RenderPassDescriptor::new("loop", RenderDestination::Target(target))
                    .with_reads(&[target]),
            )
            .is_err()
    );
}

#[test]
fn the_camera_position_travels_per_pass() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let mirror = renderer
        .add_pass(
            &RenderPassDescriptor::new("mirror", RenderDestination::Target(target))
                .with_camera_position(Vec3::new(0.0, 7.0, 0.0))
                .with_order(-1),
        )
        .unwrap();
    renderer.set_camera_position(Vec3::new(1.0, 2.0, 3.0));
    renderer.render_frame().unwrap();
    let lines = render_lines(&journal);
    assert!(lines[0].ends_with(" cam=(0, 7, 0)"));
    assert!(lines[1].ends_with(" cam=(1, 2, 3)"));

    // Le setter par passe écrase ; ZERO = pas de suffixe (le défaut).
    renderer
        .set_pass_camera_position(mirror, Vec3::ZERO)
        .unwrap();
    renderer.set_camera_position(Vec3::ZERO);
    renderer.render_frame().unwrap();
    let lines = render_lines(&journal);
    assert!(!lines[2].contains("cam="));
    assert!(!lines[3].contains("cam="));

    // Le rendu immédiat n'a pas de caméra (documenté : ZERO).
    renderer
        .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[])
        .unwrap();
    assert!(!render_lines(&journal).pop().unwrap().contains("cam="));
}
