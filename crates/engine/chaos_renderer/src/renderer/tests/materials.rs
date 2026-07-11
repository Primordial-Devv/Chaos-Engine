//! Le système de materials : bindings, permutations de pipeline partagées,
//! modèles custom, écritures in-place et inspection.

use super::*;

#[test]
fn create_material_forwards_texture_sampler_and_color() {
    let (mut renderer, journal) = mock_renderer();
    let (texture, sampler) = texture_and_sampler(&mut renderer);
    renderer
        .create_material(
            &MaterialDescriptor::new("m", MaterialModel::Unlit)
                .with_base_color(Color::rgb(0.5, 0.25, 1.0))
                .with_texture(texture)
                .with_sampler(sampler),
        )
        .unwrap();
    let entries = journal.entries();
    assert_eq!(
        entries[entries.len() - 1],
        "create_material_binding m texture=0 sampler=0 color=(0.5, 0.25, 1, 1) mr=1 normal=2 ao=1 em=1"
    );
}

#[test]
fn destroy_material_destroys_its_binding() {
    let (mut renderer, journal) = mock_renderer();
    let material = plain_material(&mut renderer, "p");
    renderer.destroy_material(material).unwrap();
    renderer.render_frame().unwrap();
    let entries = journal.entries();
    assert_eq!(
        entries[entries.len() - 1],
        "destroy_material_binding index=0"
    );
    let error = renderer.destroy_material(material).unwrap_err();
    assert!(error.to_string().contains("stale"));
}

#[test]
fn same_model_and_state_share_one_pipeline() {
    let (mut renderer, journal) = mock_renderer();
    plain_material(&mut renderer, "a");
    plain_material(&mut renderer, "b");
    assert_eq!(create_pipeline_lines(&journal).len(), 1);
    assert!(
        create_pipeline_lines(&journal)[0]
            .starts_with("create_pipeline chaos.material.vertex_color ")
    );
}

#[test]
fn each_state_is_its_own_permutation() {
    let (mut renderer, journal) = mock_renderer();
    renderer
        .create_material(&MaterialDescriptor::new(
            "culled",
            MaterialModel::VertexColor,
        ))
        .unwrap();
    renderer
        .create_material(
            &MaterialDescriptor::new("flat", MaterialModel::VertexColor).double_sided(),
        )
        .unwrap();
    renderer
        .create_material(
            &MaterialDescriptor::new("glass", MaterialModel::VertexColor)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
    let lines = create_pipeline_lines(&journal);
    assert_eq!(lines.len(), 3);
    assert!(lines[1].contains("chaos.material.vertex_color.double_sided"));
    assert!(lines[2].contains("chaos.material.vertex_color.transparent"));
    assert!(lines[2].ends_with(" blend=alpha"));
}

#[test]
fn one_material_serves_surface_and_target_passes() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let material = plain_material(&mut renderer, "scene");
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    let mirror = renderer
        .add_pass(
            &RenderPassDescriptor::new("mirror", RenderDestination::Target(target)).with_order(-1),
        )
        .unwrap();
    let command = DrawCommand {
        mesh,
        material,
        transform: Transform::IDENTITY,
    };
    renderer.queue_draw_to(mirror, command).unwrap();
    renderer.queue_draw(command);
    renderer.render_frame().unwrap();

    // L'eager surface (pipeline 0) + la permutation cible résolue au
    // rendu (pipeline 1, target=…) — UN material, deux passes, zéro
    // duplication déclarée par le consommateur.
    let pipelines = create_pipeline_lines(&journal);
    assert_eq!(pipelines.len(), 2);
    assert!(pipelines[1].contains("target=Rgba8UnormSrgb"));
    let lines = render_lines(&journal);
    assert!(lines[0].contains("dest=target0"));
    assert!(lines[0].contains("draws=[(1,"));
    assert!(lines[1].contains("draws=[(0,"));
}

#[test]
fn a_custom_model_resolves_or_fails_at_creation() {
    let (mut renderer, journal) = mock_renderer();
    renderer
        .create_material(&MaterialDescriptor::new(
            "toon",
            MaterialModel::Custom {
                shader: ShaderRef::Inline(ShaderSource::Wgsl(String::from("custom-code"))),
                vertex_layout: ColorVertex::layout(),
                material_inputs: false,
            },
        ))
        .unwrap();
    assert!(
        create_pipeline_lines(&journal)[0]
            .starts_with("create_pipeline chaos.material.custom.inline code=custom-code")
    );

    let missing = renderer
        .create_material(&MaterialDescriptor::new(
            "broken",
            MaterialModel::Custom {
                shader: ShaderRef::from("game.missing"),
                vertex_layout: ColorVertex::layout(),
                material_inputs: false,
            },
        ))
        .unwrap_err();
    assert!(missing.to_string().contains("not found in the library"));
}

#[test]
fn a_mismatched_vertex_layout_drops_the_draw() {
    let (mut renderer, journal) = mock_renderer();
    let (texture, sampler) = texture_and_sampler(&mut renderer);
    let material = textured_material(&mut renderer, "m", texture, sampler);
    let wrong_mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    let quad_geometry = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0);
    let right_mesh = renderer
        .create_textured_mesh("quad", &quad_geometry)
        .unwrap();
    renderer.queue_draw(DrawCommand {
        mesh: wrong_mesh,
        material,
        transform: Transform::IDENTITY,
    });
    renderer.queue_draw(DrawCommand {
        mesh: right_mesh,
        material,
        transform: Transform::IDENTITY,
    });
    renderer.render_frame().unwrap();
    // Le mesh ColorVertex est écarté (le modèle Unlit attend
    // TexturedVertex), le mesh assorti passe.
    let line = render_lines(&journal).pop().unwrap();
    assert_eq!(line.matches("m=[").count(), 1);
    assert_eq!(renderer.frame_report().passes[0].draws, 1);
}

#[test]
fn inputs_on_an_inputless_model_are_refused() {
    let (mut renderer, _journal) = mock_renderer();
    let (texture, sampler) = texture_and_sampler(&mut renderer);
    let with_texture = renderer
        .create_material(
            &MaterialDescriptor::new("t", MaterialModel::VertexColor).with_texture(texture),
        )
        .unwrap_err();
    assert!(with_texture.to_string().contains("no material inputs"));
    let with_sampler = renderer
        .create_material(
            &MaterialDescriptor::new("s", MaterialModel::VertexColor).with_sampler(sampler),
        )
        .unwrap_err();
    assert!(with_sampler.to_string().contains("no material inputs"));
    let with_color = renderer
        .create_material(
            &MaterialDescriptor::new("c", MaterialModel::VertexColor)
                .with_base_color(Color::rgb(1.0, 0.0, 0.0)),
        )
        .unwrap_err();
    assert!(with_color.to_string().contains("base_color"));

    let plain = plain_material(&mut renderer, "p");
    let set_color = renderer
        .set_material_color(plain, Color::rgb(1.0, 0.0, 0.0))
        .unwrap_err();
    assert!(set_color.to_string().contains("no material inputs"));
    let set_texture = renderer
        .set_material_texture(plain, Some(texture))
        .unwrap_err();
    assert!(set_texture.to_string().contains("no material inputs"));
}

#[test]
fn set_material_color_writes_in_place() {
    let (mut renderer, journal) = mock_renderer();
    let (texture, sampler) = texture_and_sampler(&mut renderer);
    let material = textured_material(&mut renderer, "m", texture, sampler);
    let bindings_before = journal
        .entries()
        .iter()
        .filter(|entry| entry.starts_with("create_material_binding"))
        .count();
    let pipelines_before = create_pipeline_lines(&journal).len();

    renderer
        .set_material_color(material, Color::rgb(0.9, 0.1, 0.2))
        .unwrap();

    let entries = journal.entries();
    assert_eq!(
        entries[entries.len() - 1],
        "update_material_binding index=0 color=(0.9, 0.1, 0.2, 1)"
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.starts_with("create_material_binding"))
            .count(),
        bindings_before
    );
    assert_eq!(create_pipeline_lines(&journal).len(), pipelines_before);
    let info = renderer.material_info(material).unwrap();
    assert_eq!(info.base_color, Color::rgb(0.9, 0.1, 0.2));

    renderer.destroy_material(material).unwrap();
    let stale = renderer
        .set_material_color(material, Color::WHITE)
        .unwrap_err();
    assert!(stale.to_string().contains("stale"));
}

#[test]
fn set_material_texture_swaps_transactionally() {
    let (mut renderer, journal) = mock_renderer();
    let first = small_texture(&mut renderer, "first");
    let second = small_texture(&mut renderer, "second");
    let sampler = renderer
        .create_sampler(&SamplerDescriptor::new("s"))
        .unwrap();
    let material = textured_material(&mut renderer, "m", first, sampler);

    renderer
        .set_material_texture(material, Some(second))
        .unwrap();

    // L'ancienne texture est rendue (destructible), la nouvelle est
    // partagée (refusée), le handle du material SURVIT.
    renderer.destroy_texture(first).unwrap();
    let still_used = renderer.destroy_texture(second).unwrap_err();
    assert!(still_used.to_string().contains("1 material(s)"));
    assert_eq!(renderer.material_info(material).unwrap().texture, second);

    // L'ancien binding part en retraite, vidée au point sûr.
    renderer.render_frame().unwrap();
    assert!(
        journal
            .entries()
            .contains(&String::from("destroy_material_binding index=0"))
    );

    // La même texture est un no-op : aucun nouveau binding.
    let bindings_before = journal
        .entries()
        .iter()
        .filter(|entry| entry.starts_with("create_material_binding"))
        .count();
    renderer
        .set_material_texture(material, Some(second))
        .unwrap();
    assert_eq!(
        journal
            .entries()
            .iter()
            .filter(|entry| entry.starts_with("create_material_binding"))
            .count(),
        bindings_before
    );

    // Un cubemap reste refusé au même titre qu'à la création.
    let cube = renderer
        .create_texture(&TextureDescriptor::cube(
            "env",
            1,
            TextureFormat::Rgba8Unorm,
            vec![0; 24],
        ))
        .unwrap();
    let refused = renderer
        .set_material_texture(material, Some(cube))
        .unwrap_err();
    assert!(refused.to_string().contains("cubemap"));
}

#[test]
fn material_info_is_the_full_inspection() {
    let (mut renderer, _journal) = mock_renderer();
    let (texture, sampler) = texture_and_sampler(&mut renderer);
    let material = renderer
        .create_material(
            &MaterialDescriptor::new("inspected", MaterialModel::Unlit)
                .double_sided()
                .with_opacity(MaterialOpacity::Transparent)
                .with_base_color(Color::rgb(0.5, 0.25, 1.0))
                .with_texture(texture)
                .with_sampler(sampler),
        )
        .unwrap();
    let info = renderer.material_info(material).unwrap();
    assert_eq!(info.label, "inspected");
    assert_eq!(info.model, MaterialModel::Unlit);
    assert_eq!(info.base_color, Color::rgb(0.5, 0.25, 1.0));
    assert_eq!(info.texture, texture);
    assert_eq!(info.sampler, sampler);
    assert!(info.double_sided);
    assert_eq!(info.opacity, MaterialOpacity::Transparent);

    renderer.destroy_material(material).unwrap();
    assert!(
        renderer
            .material_info(material)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
}

#[test]
fn stats_count_lazy_permutations_and_bindings() {
    let (mut renderer, _journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let material = plain_material(&mut renderer, "scene");
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    let stats = renderer.resource_stats();
    assert_eq!(stats.pipelines, 1);
    assert_eq!(stats.materials, 1);
    assert_eq!(stats.bindings, 1);

    let mirror = renderer
        .add_pass(
            &RenderPassDescriptor::new("mirror", RenderDestination::Target(target)).with_order(-1),
        )
        .unwrap();
    renderer
        .queue_draw_to(
            mirror,
            DrawCommand {
                mesh,
                material,
                transform: Transform::IDENTITY,
            },
        )
        .unwrap();
    renderer.render_frame().unwrap();
    // La permutation cible résolue au rendu est comptée aux stats.
    assert_eq!(renderer.resource_stats().pipelines, 2);
}

#[test]
fn a_feedback_introduced_by_update_is_caught() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let texture = small_texture(&mut renderer, "innocent");
    let material = renderer
        .create_material(
            &MaterialDescriptor::new("screen", MaterialModel::Unlit).with_texture(texture),
        )
        .unwrap();
    let quad_geometry = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0);
    let mesh = renderer
        .create_textured_mesh("quad", &quad_geometry)
        .unwrap();
    let mirror = renderer
        .add_pass(
            &RenderPassDescriptor::new("mirror", RenderDestination::Target(target)).with_order(-1),
        )
        .unwrap();
    renderer
        .queue_draw_to(
            mirror,
            DrawCommand {
                mesh,
                material,
                transform: Transform::IDENTITY,
            },
        )
        .unwrap();
    renderer.render_frame().unwrap();
    assert_eq!(renderer.frame_report().passes[0].draws, 1);

    // Le material se met à échantillonner la cible de SA passe : le
    // resolve suivant l'écarte.
    let color = renderer.render_target_color(target).unwrap();
    renderer
        .set_material_texture(material, Some(color))
        .unwrap();
    renderer.render_frame().unwrap();
    assert_eq!(renderer.frame_report().passes[0].draws, 0);
    let _ = journal;
}

#[test]
fn the_material_system_checkpoint() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let checker = small_texture(&mut renderer, "checker");
    // Deux meshes partagent le material A ; un troisième porte le
    // material B ; un quatrième au MAUVAIS layout est écarté.
    let shared = renderer
        .create_material(
            &MaterialDescriptor::new("shared", MaterialModel::Unlit).with_texture(checker),
        )
        .unwrap();
    let tinted = renderer
        .create_material(
            &MaterialDescriptor::new("tinted", MaterialModel::Unlit)
                .with_base_color(Color::rgb(0.2, 0.4, 0.8)),
        )
        .unwrap();
    let quad_geometry = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0);
    let mesh_a = renderer.create_textured_mesh("a", &quad_geometry).unwrap();
    let mesh_b = renderer.create_textured_mesh("b", &quad_geometry).unwrap();
    let mesh_c = renderer.create_textured_mesh("c", &quad_geometry).unwrap();
    let wrong = renderer.create_mesh("wrong", &triangle()).unwrap();
    let mirror = renderer
        .add_pass(
            &RenderPassDescriptor::new("mirror", RenderDestination::Target(target)).with_order(-10),
        )
        .unwrap();

    for _ in 0..2 {
        renderer.clear_draws();
        for mesh in [mesh_a, mesh_b] {
            renderer.queue_draw(DrawCommand {
                mesh,
                material: shared,
                transform: Transform::IDENTITY,
            });
        }
        renderer.queue_draw(DrawCommand {
            mesh: mesh_c,
            material: tinted,
            transform: Transform::IDENTITY,
        });
        renderer.queue_draw(DrawCommand {
            mesh: wrong,
            material: tinted,
            transform: Transform::IDENTITY,
        });
        renderer
            .queue_draw_to(
                mirror,
                DrawCommand {
                    mesh: mesh_a,
                    material: shared,
                    transform: Transform::IDENTITY,
                },
            )
            .unwrap();
        renderer.render_frame().unwrap();
        // Entre les deux frames : la couleur de B change SANS
        // recréation (le chemin contrôlé).
        renderer
            .set_material_color(tinted, Color::rgb(0.9, 0.9, 0.1))
            .unwrap();
    }

    let report = renderer.frame_report();
    assert_eq!(report.passes[0].label, "mirror");
    assert_eq!(report.passes[0].draws, 1);
    assert_eq!(report.passes[1].label, "chaos.main");
    assert_eq!(report.passes[1].draws, 3);

    let entries = journal.entries();
    // Deux materials Unlit au même état = UNE permutation surface +
    // UNE permutation cible ; deux bindings ; deux updates in-place ;
    // aucun binding recréé.
    assert_eq!(create_pipeline_lines(&journal).len(), 2);
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.starts_with("create_material_binding"))
            .count(),
        2
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.starts_with("update_material_binding"))
            .count(),
        2
    );
}
