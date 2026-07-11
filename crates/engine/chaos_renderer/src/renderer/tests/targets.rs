//! Les render targets : création, couleur partagée, resize rotatif,
//! `render_to_target` et le flux offscreen → écran complet.

use super::*;

#[test]
fn a_render_target_reaches_the_backend_and_the_stats() {
    let (mut renderer, journal) = mock_renderer();
    small_target(&mut renderer, "viewport");
    assert_eq!(
        journal.entries(),
        vec!["create_render_target viewport 4x4 format=Rgba8UnormSrgb"]
    );
    let stats = renderer.resource_stats();
    assert_eq!(stats.render_targets.alive, 1);
    assert_eq!(stats.render_targets.bytes, 4 * 4 * 4);
    assert_eq!(stats.textures.alive, 1);
    assert_eq!(stats.textures.bytes, 4 * 4 * 4);
}

#[test]
fn the_target_color_feeds_a_material() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let color = renderer.render_target_color(target).unwrap();
    renderer
        .create_material(
            &MaterialDescriptor::new("screen", MaterialModel::Unlit).with_texture(color),
        )
        .unwrap();
    assert!(
        journal
            .entries()
            .iter()
            .any(|entry| entry.starts_with("create_material_binding screen texture=0"))
    );
}

#[test]
fn render_to_target_orders_resolves_and_targets() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let material = plain_material(&mut renderer, "p");
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    renderer.queue_draw(DrawCommand {
        mesh,
        material,
        transform: Transform::IDENTITY,
    });
    let main_draws_before = renderer.draw_count();
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
    assert_eq!(renderer.draw_count(), main_draws_before);
    let entries = journal.entries();
    let target_render = entries
        .iter()
        .find(|entry| entry.contains("dest=target0"))
        .unwrap();
    // La permutation surface (pipeline 0, l'eager) et la permutation
    // cible (pipeline 1) sont deux pipelines distincts.
    assert!(target_render.contains("draws=[(1,"));
}

#[test]
fn stale_target_operations_are_explicit_errors() {
    let (mut renderer, _journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    renderer.destroy_render_target(target).unwrap();
    assert!(
        renderer
            .render_target_color(target)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert!(
        renderer
            .render_target_size(target)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert!(
        renderer
            .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[])
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert!(
        renderer
            .destroy_render_target(target)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
}

#[test]
fn stale_draws_are_dropped_from_target_passes_too() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let material = plain_material(&mut renderer, "p");
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    let command = DrawCommand {
        mesh,
        material,
        transform: Transform::IDENTITY,
    };
    renderer.destroy_mesh(mesh).unwrap();
    renderer
        .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[command])
        .unwrap();
    assert!(
        journal
            .entries()
            .iter()
            .any(|entry| entry.contains("draws=[] dest=target0"))
    );
}

#[test]
fn resize_rotates_the_handles() {
    let (mut renderer, _journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let old_color = renderer.render_target_color(target).unwrap();
    let resized = renderer.resize_render_target(target, 8, 8).unwrap();
    assert_ne!(target, resized);
    assert_eq!(renderer.render_target_size(resized).unwrap(), (8, 8));
    assert!(renderer.render_target_color(target).is_err());
    let new_color = renderer.render_target_color(resized).unwrap();
    assert_ne!(old_color, new_color);
    assert!(renderer.resource_stats().retired > 0);
    renderer.render_frame().unwrap();
    assert_eq!(renderer.resource_stats().retired, 0);
}

#[test]
fn destroying_a_target_is_refused_while_its_color_is_shared() {
    let (mut renderer, _journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let color = renderer.render_target_color(target).unwrap();
    let material = renderer
        .create_material(
            &MaterialDescriptor::new("screen", MaterialModel::Unlit).with_texture(color),
        )
        .unwrap();
    let error = renderer.destroy_render_target(target).unwrap_err();
    assert!(error.to_string().contains("still used by 1 material(s)"));
    renderer.destroy_material(material).unwrap();
    renderer.destroy_render_target(target).unwrap();
}

#[test]
fn target_stats_return_to_baseline() {
    let (mut renderer, _journal) = mock_renderer();
    let baseline = renderer.resource_stats();
    let target = small_target(&mut renderer, "viewport");
    renderer.destroy_render_target(target).unwrap();
    renderer.render_frame().unwrap();
    let stats = renderer.resource_stats();
    assert_eq!(stats.render_targets, baseline.render_targets);
    assert_eq!(stats.textures, baseline.textures);
    assert_eq!(stats.retired, 0);
}

#[test]
fn a_pipeline_color_target_reaches_the_backend() {
    let (mut renderer, journal) = mock_renderer();
    renderer
        .create_pipeline(
            &inline_descriptor("offscreen").with_color_target(TextureFormat::Rgba16Float),
        )
        .unwrap();
    assert!(
        journal
            .entries()
            .iter()
            .any(|entry| entry.contains("target=Rgba16Float"))
    );
}

#[test]
fn offscreen_then_display_is_the_full_checkpoint_flow() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    // UN seul material de scène : la permutation offscreen se résout
    // seule au moment du rendu vers la cible — plus de duplication.
    let scene_material = plain_material(&mut renderer, "scene");
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    let screen_quad = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0);
    let screen_mesh = renderer
        .create_textured_mesh("screen", &screen_quad)
        .unwrap();
    renderer
        .render_to_target(
            target,
            Color::BLACK,
            Mat4::IDENTITY,
            &[DrawCommand {
                mesh,
                material: scene_material,
                transform: Transform::IDENTITY,
            }],
        )
        .unwrap();

    let color = renderer.render_target_color(target).unwrap();
    let screen_material = renderer
        .create_material(
            &MaterialDescriptor::new("screen", MaterialModel::Unlit).with_texture(color),
        )
        .unwrap();
    renderer.queue_draw(DrawCommand {
        mesh: screen_mesh,
        material: screen_material,
        transform: Transform::IDENTITY,
    });
    renderer.render_frame().unwrap();

    renderer.destroy_material(screen_material).unwrap();
    let resized = renderer.resize_render_target(target, 8, 8).unwrap();
    let new_color = renderer.render_target_color(resized).unwrap();
    let screen_material = renderer
        .create_material(
            &MaterialDescriptor::new("screen", MaterialModel::Unlit).with_texture(new_color),
        )
        .unwrap();
    renderer
        .render_to_target(
            resized,
            Color::BLACK,
            Mat4::IDENTITY,
            &[DrawCommand {
                mesh,
                material: scene_material,
                transform: Transform::IDENTITY,
            }],
        )
        .unwrap();
    renderer.clear_draws();
    renderer.queue_draw(DrawCommand {
        mesh: screen_mesh,
        material: screen_material,
        transform: Transform::IDENTITY,
    });
    renderer.render_frame().unwrap();

    let entries = journal.entries();
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.contains("dest=target"))
            .count(),
        2
    );
    assert!(entries.iter().any(|entry| entry.contains("dest=target1")));
    // Les deux passes cible dessinent : la permutation offscreen du
    // material de scène a bien été résolue (jamais un draw vide).
    assert!(
        entries
            .iter()
            .filter(|entry| entry.contains("dest=target"))
            .all(|entry| entry.contains("draws=[("))
    );
}
