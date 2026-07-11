//! L'éclairage : géométrie et materials éclairés, collection des lumières,
//! ambiante, PBR et ses slots, environnement, ciel et exposition.

use super::*;

#[test]
fn the_lit_vertex_declares_its_layout() {
    let layout = LitVertex::layout();
    assert_eq!(layout.stride, 32);
    assert_eq!(layout.attributes.len(), 3);
    assert_eq!(layout.attributes[1].offset, 12);
    assert_eq!(layout.attributes[2].offset, 24);
    let vertex = LitVertex {
        position: [1.0, 2.0, 3.0],
        normal: [0.0, 1.0, 0.0],
        uv: [0.5, 0.25],
    };
    let bytes = LitVertex::bytes_of(&[vertex]);
    assert_eq!(bytes.len(), 32);
    assert_eq!(bytes[12..16], 0.0f32.to_ne_bytes());
    assert_eq!(bytes[16..20], 1.0f32.to_ne_bytes());
}

#[test]
fn lit_geometry_keeps_its_face_normals() {
    let cube = LitGeometry::cube([0.0, 0.0, 0.0], 2.0);
    assert_eq!(cube.vertices.len(), 24);
    assert_eq!(cube.indices.len(), 36);
    for vertex in &cube.vertices {
        let length: f32 = vertex
            .normal
            .iter()
            .map(|component| component * component)
            .sum();
        assert!((length - 1.0).abs() < 1e-6);
    }
    // La face +Y (sommets 8..12) porte la normale +Y exacte.
    assert_eq!(cube.vertices[8].normal, [0.0, 1.0, 0.0]);
    let quad = LitGeometry::quad([0.0, 0.0, 0.0], 2.0, 2.0, 1.0);
    for vertex in &quad.vertices {
        assert_eq!(vertex.normal, [0.0, 0.0, 1.0]);
    }
}

#[test]
fn a_lit_material_resolves_the_lit_permutation() {
    let (mut renderer, journal) = mock_renderer();
    let material = renderer
        .create_material(&MaterialDescriptor::new("shaded", MaterialModel::Lit))
        .unwrap();
    assert!(create_pipeline_lines(&journal)[0].starts_with("create_pipeline chaos.material.lit "));
    let mesh = lit_quad_mesh(&mut renderer, "quad");
    renderer.queue_draw(DrawCommand {
        mesh,
        material,
        transform: Transform::IDENTITY,
    });
    renderer.render_frame().unwrap();
    let line = render_lines(&journal).pop().unwrap();
    assert_eq!(line.matches("m=[").count(), 1);
}

#[test]
fn a_textured_mesh_under_a_lit_material_is_dropped() {
    let (mut renderer, journal) = mock_renderer();
    let material = renderer
        .create_material(&MaterialDescriptor::new("shaded", MaterialModel::Lit))
        .unwrap();
    let quad = TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0);
    let wrong = renderer.create_textured_mesh("quad", &quad).unwrap();
    renderer.queue_draw(DrawCommand {
        mesh: wrong,
        material,
        transform: Transform::IDENTITY,
    });
    renderer.render_frame().unwrap();
    let line = render_lines(&journal).pop().unwrap();
    assert!(line.ends_with("draws=[]"));
}

#[test]
fn submitted_lights_reach_the_backend() {
    let (mut renderer, journal) = mock_renderer();
    renderer.set_ambient_light(Color::rgb(1.0, 1.0, 1.0), 0.1);
    renderer.submit_light(Light::directional(
        Vec3::new(0.0, -2.0, 0.0),
        Color::rgb(1.0, 0.9, 0.8),
        0.9,
    ));
    renderer.submit_light(Light::point(
        Vec3::new(1.0, 2.0, 3.0),
        Color::rgb(1.0, 0.0, 0.0),
        2.5,
        5.0,
    ));
    renderer.render_frame().unwrap();
    let lines = lights_lines(&journal);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("lights ambient=(1, 1, 1, 0.1) count=2"));
    // La direction soumise (0, -2, 0) arrive NORMALISÉE au backend.
    assert!(lines[0].contains("[directional d=(0, -1, 0) c=(1, 0.9, 0.8) i=0.9]"));
    assert!(lines[0].contains("[point p=(1, 2, 3) r=5 c=(1, 0, 0) i=2.5]"));
}

#[test]
fn an_unlit_frame_emits_no_lights_line() {
    let (mut renderer, journal) = mock_renderer();
    renderer.render_frame().unwrap();
    assert!(lights_lines(&journal).is_empty());
}

#[test]
fn lights_overflow_is_truncated_predictably() {
    let (mut renderer, journal) = mock_renderer();
    for index in 0..20 {
        renderer.submit_light(Light::point(
            Vec3::new(f32::from(u8::try_from(index).unwrap_or(0)), 0.0, 0.0),
            Color::WHITE,
            1.0,
            5.0,
        ));
    }
    renderer.render_frame().unwrap();
    let line = lights_lines(&journal).pop().unwrap();
    assert!(line.contains("count=16"));
    // L'ordre de soumission est préservé : la première gagne, la
    // dix-septième (x=16) n'entre pas.
    assert!(line.contains("[point p=(0, 0, 0)"));
    assert!(line.contains("p=(15, 0, 0)"));
    assert!(!line.contains("p=(16, 0, 0)"));
    // Sous la limite à la frame suivante : l'épisode se réarme.
    renderer.clear_draws();
    renderer.submit_light(Light::point(Vec3::ZERO, Color::WHITE, 1.0, 5.0));
    renderer.render_frame().unwrap();
    assert!(lights_lines(&journal).pop().unwrap().contains("count=1"));
    assert!(!renderer.lights_truncation_warned);
}

#[test]
fn disabled_and_invalid_lights_are_excluded() {
    let (mut renderer, journal) = mock_renderer();
    let mut off = Light::point(Vec3::ZERO, Color::WHITE, 1.0, 5.0);
    off.set_enabled(false);
    renderer.submit_light(off);
    // Invalides : écartées AU SUBMIT, jamais stockées.
    renderer.submit_light(Light::directional(Vec3::ZERO, Color::WHITE, 1.0));
    renderer.submit_light(Light::point(Vec3::ZERO, Color::WHITE, -1.0, 5.0));
    renderer.submit_light(Light::spot(
        Vec3::ZERO,
        Vec3::NEG_Y,
        Color::WHITE,
        1.0,
        5.0,
        0.4,
        0.4,
    ));
    renderer.submit_light(Light::point(Vec3::X, Color::WHITE, 1.0, 5.0));
    renderer.render_frame().unwrap();
    let line = lights_lines(&journal).pop().unwrap();
    assert!(line.contains("count=1"));
    assert!(line.contains("p=(1, 0, 0)"));
}

#[test]
fn clear_draws_clears_lights_but_keeps_the_ambient() {
    let (mut renderer, journal) = mock_renderer();
    renderer.set_ambient_light(Color::WHITE, 0.2);
    renderer.submit_light(Light::point(Vec3::ZERO, Color::WHITE, 1.0, 5.0));
    renderer.render_frame().unwrap();
    assert!(lights_lines(&journal).pop().unwrap().contains("count=1"));

    renderer.clear_draws();
    renderer.render_frame().unwrap();
    // Les lumières sont re-soumises chaque frame ; l'ambiante est un
    // réglage persistant.
    let line = lights_lines(&journal).pop().unwrap();
    assert!(line.contains("count=0"));
    assert!(line.contains("ambient=(1, 1, 1, 0.2)"));
    assert_eq!(renderer.ambient_light(), (Color::WHITE, 0.2));
}

#[test]
fn the_frame_lights_serve_every_pass() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    renderer
        .add_pass(
            &RenderPassDescriptor::new("mirror", RenderDestination::Target(target)).with_order(-1),
        )
        .unwrap();
    renderer.submit_light(Light::point(Vec3::ZERO, Color::WHITE, 1.0, 5.0));
    renderer.render_frame().unwrap();
    // UNE ligne lights pour le plan entier : les deux passes
    // partagent le même éclairage.
    assert_eq!(lights_lines(&journal).len(), 1);
    assert_eq!(render_lines(&journal).len(), 2);
}

#[test]
fn render_to_target_carries_the_collected_lights() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let mut off = Light::point(Vec3::X, Color::WHITE, 1.0, 5.0);
    off.set_enabled(false);
    renderer.submit_light(off);
    renderer.submit_light(Light::point(Vec3::ZERO, Color::WHITE, 1.0, 5.0));
    renderer
        .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[])
        .unwrap();
    // Le chemin immédiat passe par la MÊME collection : la
    // désactivée est filtrée là aussi.
    let line = lights_lines(&journal).pop().unwrap();
    assert!(line.contains("count=1"));
    assert!(line.contains("p=(0, 0, 0)"));
}

#[test]
fn an_empty_plan_sends_no_lights() {
    let (mut renderer, journal) = mock_renderer();
    renderer
        .set_pass_enabled(renderer.main_pass(), false)
        .unwrap();
    renderer.submit_light(Light::point(Vec3::ZERO, Color::WHITE, 1.0, 5.0));
    renderer.render_frame().unwrap();
    assert!(lights_lines(&journal).is_empty());
    assert!(render_lines(&journal).is_empty());
}

#[test]
fn the_lighting_checkpoint() {
    let (mut renderer, journal) = mock_renderer();
    // La scène de validation : un material Lit sur mesh éclairable,
    // une directionnelle + deux ponctuelles + un spot.
    let (texture, sampler) = texture_and_sampler(&mut renderer);
    let material = renderer
        .create_material(
            &MaterialDescriptor::new("shaded", MaterialModel::Lit)
                .with_texture(texture)
                .with_sampler(sampler),
        )
        .unwrap();
    let mesh = lit_quad_mesh(&mut renderer, "quad");
    renderer.set_ambient_light(Color::WHITE, 0.05);

    for frame in 0..2 {
        renderer.clear_draws();
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
        let mut sun = Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0);
        // La deuxième frame éteint le soleil : le toggle observable.
        sun.set_enabled(frame == 0);
        renderer.submit_light(sun);
        renderer.submit_light(Light::point(Vec3::X, Color::rgb(1.0, 0.0, 0.0), 2.0, 4.0));
        renderer.submit_light(Light::point(Vec3::Z, Color::rgb(0.0, 0.0, 1.0), 2.0, 4.0));
        renderer.submit_light(Light::spot(
            Vec3::Y,
            Vec3::NEG_Y,
            Color::WHITE,
            3.0,
            8.0,
            0.2,
            0.4,
        ));
        renderer.render_frame().unwrap();
    }

    let lines = lights_lines(&journal);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("count=4"));
    assert!(lines[0].contains("[directional"));
    assert!(lines[0].contains("[spot p=(0, 1, 0) d=(0, -1, 0) r=8]"));
    assert!(lines[1].contains("count=3"));
    assert!(!lines[1].contains("[directional"));
    // Le draw éclairé est résolu dans les deux frames.
    for line in render_lines(&journal) {
        assert_eq!(line.matches("m=[").count(), 1);
    }
}

#[test]
fn pbr_properties_are_refused_on_non_pbr_models() {
    let (mut renderer, _journal) = mock_renderer();
    let refused = renderer
        .create_material(&MaterialDescriptor::new("m", MaterialModel::Lit).with_metallic(0.5))
        .unwrap_err();
    assert!(refused.to_string().contains("does not consume PBR"));
    assert!(refused.to_string().contains("'metallic'"));
    let (texture, _sampler) = texture_and_sampler(&mut renderer);
    let mapped = renderer
        .create_material(
            &MaterialDescriptor::new("m", MaterialModel::Unlit).with_normal_map(texture),
        )
        .unwrap_err();
    assert!(mapped.to_string().contains("'normal_map'"));

    // Acceptées sur Pbr et sur Custom-avec-inputs (le shader custom
    // voit tout le groupe 2 — délégation documentée).
    renderer
        .create_material(
            &MaterialDescriptor::new("p", MaterialModel::Pbr)
                .with_metallic(0.5)
                .with_roughness(0.3),
        )
        .unwrap();
    renderer
        .create_material(
            &MaterialDescriptor::new(
                "c",
                MaterialModel::Custom {
                    shader: ShaderRef::Inline(ShaderSource::Wgsl(String::from("custom-code"))),
                    vertex_layout: LitVertex::layout(),
                    material_inputs: true,
                },
            )
            .with_emissive(Color::rgb(1.0, 0.0, 0.0)),
        )
        .unwrap();
}

#[test]
fn a_pbr_material_resolves_its_permutation_and_draws() {
    let (mut renderer, journal) = mock_renderer();
    let material = renderer
        .create_material(&MaterialDescriptor::new("shaded", MaterialModel::Pbr))
        .unwrap();
    assert!(create_pipeline_lines(&journal)[0].starts_with("create_pipeline chaos.material.pbr "));
    let mesh = lit_quad_mesh(&mut renderer, "quad");
    renderer.queue_draw(DrawCommand {
        mesh,
        material,
        transform: Transform::IDENTITY,
    });
    renderer.render_frame().unwrap();
    let line = render_lines(&journal).pop().unwrap();
    assert_eq!(line.matches("m=[").count(), 1);
}

#[test]
fn the_binding_carries_the_seven_slots() {
    let (mut renderer, journal) = mock_renderer();
    let mr = small_texture(&mut renderer, "mr");
    let normal = small_texture(&mut renderer, "bumps");
    renderer
        .create_material(
            &MaterialDescriptor::new("full", MaterialModel::Pbr)
                .with_metallic(0.7)
                .with_roughness(0.25)
                .with_metallic_roughness_texture(mr)
                .with_normal_map(normal)
                .with_emissive(Color::rgb(2.0, 1.0, 0.5)),
        )
        .unwrap();
    let line = binding_lines(&journal).pop().unwrap();
    // mr/normal explicites (idx 0 et 1), ao/émissif en fallback
    // blanc (idx 2), base en fallback blanc aussi.
    assert!(line.contains("texture=2"));
    assert!(line.contains(" mr=0 "));
    assert!(line.contains(" normal=1 "));
    assert!(line.contains(" ao=2 "));
    assert!(line.contains(" em=2"));
    assert!(line.contains(" metallic=0.7"));
    assert!(line.contains(" roughness=0.25"));
    assert!(line.contains(" emissive=(2, 1, 0.5)"));
}

#[test]
fn cubemaps_are_refused_on_every_pbr_slot() {
    let (mut renderer, _journal) = mock_renderer();
    let cube = renderer
        .create_texture(&TextureDescriptor::cube(
            "env",
            1,
            TextureFormat::Rgba8Unorm,
            vec![0; 24],
        ))
        .unwrap();
    let slots: [fn(MaterialDescriptor, TextureHandle) -> MaterialDescriptor; 4] = [
        |d, t| d.with_metallic_roughness_texture(t),
        |d, t| d.with_normal_map(t),
        |d, t| d.with_occlusion_texture(t),
        |d, t| d.with_emissive_texture(t),
    ];
    for attach in slots {
        let descriptor = attach(MaterialDescriptor::new("m", MaterialModel::Pbr), cube);
        let refused = renderer.create_material(&descriptor).unwrap_err();
        assert!(refused.to_string().contains("cubemap"));
    }
}

#[test]
fn every_pbr_slot_is_refcounted() {
    let (mut renderer, _journal) = mock_renderer();
    let mr = small_texture(&mut renderer, "mr");
    let material = renderer
        .create_material(
            &MaterialDescriptor::new("m", MaterialModel::Pbr).with_metallic_roughness_texture(mr),
        )
        .unwrap();
    let refused = renderer.destroy_texture(mr).unwrap_err();
    assert!(refused.to_string().contains("still used by 1 material(s)"));
    renderer.destroy_material(material).unwrap();
    renderer.destroy_texture(mr).unwrap();
}

#[test]
fn a_doubled_slot_takes_two_shares() {
    let (mut renderer, _journal) = mock_renderer();
    let shared = small_texture(&mut renderer, "both");
    let material = renderer
        .create_material(
            &MaterialDescriptor::new("m", MaterialModel::Pbr)
                .with_texture(shared)
                .with_emissive_texture(shared),
        )
        .unwrap();
    // Deux slots = deux parts — la symétrie exacte du release.
    let refused = renderer.destroy_texture(shared).unwrap_err();
    assert!(refused.to_string().contains("still used by 2 material(s)"));
    renderer.destroy_material(material).unwrap();
    renderer.destroy_texture(shared).unwrap();
}

#[test]
fn pbr_params_update_in_place() {
    let (mut renderer, journal) = mock_renderer();
    let material = renderer
        .create_material(&MaterialDescriptor::new("m", MaterialModel::Pbr))
        .unwrap();
    let bindings_before = binding_lines(&journal).len();
    let pipelines_before = create_pipeline_lines(&journal).len();

    renderer.set_material_metallic(material, 0.9).unwrap();
    renderer.set_material_roughness(material, 0.2).unwrap();
    renderer
        .set_material_emissive(material, Color::rgb(3.0, 1.5, 0.0))
        .unwrap();

    let entries = journal.entries();
    assert_eq!(
        entries[entries.len() - 1],
        "update_material_binding index=0 color=(1, 1, 1, 1) metallic=0.9 roughness=0.2 emissive=(3, 1.5, 0)"
    );
    assert_eq!(binding_lines(&journal).len(), bindings_before);
    assert_eq!(create_pipeline_lines(&journal).len(), pipelines_before);

    let info = renderer.material_info(material).unwrap();
    assert_eq!(info.metallic, 0.9);
    assert_eq!(info.roughness, 0.2);
    assert_eq!(info.emissive, Color::rgb(3.0, 1.5, 0.0));

    // Refusé sur un modèle qui ne consomme pas les propriétés PBR.
    let lit = renderer
        .create_material(&MaterialDescriptor::new("lit", MaterialModel::Lit))
        .unwrap();
    let refused = renderer.set_material_metallic(lit, 0.5).unwrap_err();
    assert!(refused.to_string().contains("does not consume PBR"));

    renderer.destroy_material(material).unwrap();
    let stale = renderer.set_material_roughness(material, 0.5).unwrap_err();
    assert!(stale.to_string().contains("stale"));
}

#[test]
fn a_pbr_slot_feedback_is_dropped() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let color = renderer.render_target_color(target).unwrap();
    let material = renderer
        .create_material(
            &MaterialDescriptor::new("glow", MaterialModel::Pbr).with_emissive_texture(color),
        )
        .unwrap();
    let mesh = lit_quad_mesh(&mut renderer, "quad");
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
    // Le slot émissif porte la couleur de la cible de SA passe : le
    // feedback est attrapé même hors du slot de base.
    assert_eq!(renderer.frame_report().passes[0].draws, 0);
    let _ = journal;
}

#[test]
fn the_pbr_checkpoint() {
    let (mut renderer, journal) = mock_renderer();
    let normal_map = small_texture(&mut renderer, "bumps");
    let mesh = lit_quad_mesh(&mut renderer, "sphere");

    // La grille : des combinaisons metallic/roughness distinctes qui
    // partagent UNE permutation, plus une normal map et un émissif.
    let mut materials = Vec::new();
    for (index, (metallic, roughness)) in [(0.0, 0.1), (0.0, 1.0), (1.0, 0.1), (1.0, 1.0)]
        .iter()
        .enumerate()
    {
        materials.push(
            renderer
                .create_material(
                    &MaterialDescriptor::new(format!("grid.{index}"), MaterialModel::Pbr)
                        .with_metallic(*metallic)
                        .with_roughness(*roughness),
                )
                .unwrap(),
        );
    }
    let bumpy = renderer
        .create_material(
            &MaterialDescriptor::new("bumpy", MaterialModel::Pbr).with_normal_map(normal_map),
        )
        .unwrap();
    let glowing = renderer
        .create_material(
            &MaterialDescriptor::new("glowing", MaterialModel::Pbr)
                .with_emissive(Color::rgb(2.0, 0.5, 0.1)),
        )
        .unwrap();

    renderer.set_camera_position(Vec3::new(0.0, 1.0, 6.0));
    renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
    for material in materials.iter().chain([&bumpy, &glowing]) {
        renderer.queue_draw(DrawCommand {
            mesh,
            material: *material,
            transform: Transform::IDENTITY,
        });
    }
    renderer.render_frame().unwrap();

    // 6 materials PBR = 6 bindings distincts, UNE permutation.
    assert_eq!(
        create_pipeline_lines(&journal)
            .iter()
            .filter(|line| line.contains("chaos.material.pbr"))
            .count(),
        1
    );
    assert_eq!(binding_lines(&journal).len(), 6);
    let line = render_lines(&journal).pop().unwrap();
    assert_eq!(line.matches("m=[").count(), 6);
    assert!(line.ends_with(" cam=(0, 1, 6)"));

    // L'émissif pulse entre deux frames sans recréation.
    let bindings_before = binding_lines(&journal).len();
    renderer
        .set_material_emissive(glowing, Color::rgb(0.5, 0.1, 0.0))
        .unwrap();
    renderer.render_frame().unwrap();
    assert_eq!(binding_lines(&journal).len(), bindings_before);
    assert_eq!(
        renderer.material_info(glowing).unwrap().emissive,
        Color::rgb(0.5, 0.1, 0.0)
    );
}

#[test]
fn an_environment_requires_a_living_cubemap() {
    let (mut renderer, _journal) = mock_renderer();
    let flat = small_texture(&mut renderer, "flat");
    let refused = renderer
        .set_environment(&EnvironmentDescriptor::new(flat))
        .unwrap_err();
    assert!(refused.to_string().contains("'flat'"));
    assert!(refused.to_string().contains("D2"));
    assert!(refused.to_string().contains("expects a cubemap"));

    let cube = env_cubemap(&mut renderer, "sky");
    renderer.destroy_texture(cube).unwrap();
    let stale = renderer
        .set_environment(&EnvironmentDescriptor::new(cube))
        .unwrap_err();
    assert!(stale.to_string().contains("stale"));

    let alive = env_cubemap(&mut renderer, "sky2");
    let negative = renderer
        .set_environment(&EnvironmentDescriptor::new(alive).with_intensity(-1.0))
        .unwrap_err();
    assert!(negative.to_string().contains("finite and non-negative"));
    let nan = renderer
        .set_environment(&EnvironmentDescriptor::new(alive).with_intensity(f32::NAN))
        .unwrap_err();
    assert!(nan.to_string().contains("finite and non-negative"));
    assert!(renderer.environment_info().is_none());
}

#[test]
fn setting_the_environment_rebinds_the_backend_once() {
    let (mut renderer, journal) = mock_renderer();
    let cube = env_cubemap(&mut renderer, "sky");
    renderer
        .set_environment(&EnvironmentDescriptor::new(cube))
        .unwrap();
    assert_eq!(
        environment_lines(&journal),
        vec![String::from("set_environment index=0")]
    );

    // Le MÊME cubemap re-posé : intensité/ciel mis à jour, zéro rebind.
    renderer
        .set_environment(&EnvironmentDescriptor::new(cube).with_intensity(0.5))
        .unwrap();
    assert_eq!(environment_lines(&journal).len(), 1);
    assert_eq!(renderer.environment_info().unwrap().intensity, 0.5);

    // Un AUTRE cubemap rebinde.
    let other = env_cubemap(&mut renderer, "night");
    renderer
        .set_environment(&EnvironmentDescriptor::new(other))
        .unwrap();
    assert_eq!(
        environment_lines(&journal).pop().unwrap(),
        "set_environment index=1"
    );
}

#[test]
fn clearing_the_environment_rebinds_the_fallback() {
    let (mut renderer, journal) = mock_renderer();
    // Effacer sans environnement : un no-op, aucun appel backend.
    renderer.clear_environment().unwrap();
    assert!(environment_lines(&journal).is_empty());

    let cube = env_cubemap(&mut renderer, "sky");
    renderer
        .set_environment(&EnvironmentDescriptor::new(cube))
        .unwrap();
    renderer.clear_environment().unwrap();
    renderer.clear_environment().unwrap();
    assert_eq!(
        environment_lines(&journal),
        vec![
            String::from("set_environment index=0"),
            String::from("set_environment none"),
        ]
    );
    assert!(renderer.environment_info().is_none());
}

#[test]
fn the_active_environment_refuses_destruction() {
    let (mut renderer, _journal) = mock_renderer();
    let cube = env_cubemap(&mut renderer, "sky");
    renderer
        .set_environment(&EnvironmentDescriptor::new(cube))
        .unwrap();
    let refused = renderer.destroy_texture(cube).unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("'sky' is the active environment: clear it first")
    );
    renderer.clear_environment().unwrap();
    renderer.destroy_texture(cube).unwrap();
}

#[test]
fn the_sky_draw_covers_clear_passes() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    let mirror = renderer
        .add_pass(
            &RenderPassDescriptor::new("mirror", RenderDestination::Target(target)).with_order(-1),
        )
        .unwrap();
    let opaque = plain_material(&mut renderer, "solid");
    let transparent = renderer
        .create_material(
            &MaterialDescriptor::new("glass", MaterialModel::VertexColor)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    let cube = env_cubemap(&mut renderer, "sky");
    renderer
        .set_environment(&EnvironmentDescriptor::new(cube))
        .unwrap();

    for material in [opaque, transparent] {
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
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
    }
    renderer.render_frame().unwrap();

    // Chaque passe Clear dessine le ciel APRÈS ses opaques et AVANT
    // ses transparents (les draws de scène portent leurs buffers —
    // `Some(…)` — le ciel n'en a pas) ; le rapport compte le draw
    // injecté.
    for line in render_lines(&journal) {
        let sky = line.find(SKY_DRAW).unwrap();
        assert!(line[..sky].contains("Some("));
        assert!(line[sky + SKY_DRAW.len()..].contains("Some("));
    }
    for report in &renderer.frame_report().passes {
        assert_eq!(report.draws, 3);
    }

    // Le rendu immédiat vers une cible efface toujours : ciel inclus.
    renderer
        .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[])
        .unwrap();
    assert!(render_lines(&journal).pop().unwrap().contains(SKY_DRAW));

    // L'environnement effacé : le ciel disparaît de la frame suivante.
    renderer.clear_environment().unwrap();
    renderer.render_frame().unwrap();
    assert!(!render_lines(&journal).pop().unwrap().contains(SKY_DRAW));
}

#[test]
fn a_keep_pass_never_draws_the_sky() {
    let (mut renderer, journal) = mock_renderer();
    renderer
        .add_pass(
            &surface_pass("overlay")
                .with_load(PassLoad::Keep)
                .with_order(1),
        )
        .unwrap();
    let cube = env_cubemap(&mut renderer, "sky");
    renderer
        .set_environment(&EnvironmentDescriptor::new(cube))
        .unwrap();
    renderer.render_frame().unwrap();
    let lines = render_lines(&journal);
    // La principale (Clear) reçoit le ciel ; l'overlay (Keep)
    // préserve son image — jamais repeinte par le fond.
    assert!(lines[0].contains(SKY_DRAW));
    assert!(!lines[1].contains(SKY_DRAW));
    assert!(lines[1].ends_with(" load=keep"));
}

#[test]
fn the_sky_respects_its_flag() {
    let (mut renderer, journal) = mock_renderer();
    let cube = env_cubemap(&mut renderer, "sky");
    renderer
        .set_environment(&EnvironmentDescriptor::new(cube).with_sky(false))
        .unwrap();
    renderer.render_frame().unwrap();
    // Pas de draw ciel, pas de pipeline ciel — mais l'IBL voyage.
    assert!(!render_lines(&journal).pop().unwrap().contains(SKY_DRAW));
    assert!(create_pipeline_lines(&journal).is_empty());
    assert_eq!(
        journal
            .entries()
            .iter()
            .filter(|entry| entry.starts_with("environment "))
            .count(),
        1
    );
}

#[test]
fn no_environment_means_no_journal_delta() {
    let (mut renderer, journal) = mock_renderer();
    renderer.set_exposure(1.0).unwrap();
    renderer.render_frame().unwrap();
    // Sans environnement et à l'exposition par défaut : aucune ligne
    // nouvelle — le journal historique est intact.
    assert!(
        journal.entries().iter().all(
            |entry| !entry.starts_with("environment ") && !entry.starts_with("set_environment")
        )
    );
    assert!(create_pipeline_lines(&journal).is_empty());
}

#[test]
fn the_sky_pipeline_is_one_permutation_per_format() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    renderer
        .add_pass(
            &RenderPassDescriptor::new("mirror", RenderDestination::Target(target)).with_order(-1),
        )
        .unwrap();
    let cube = env_cubemap(&mut renderer, "sky");
    renderer
        .set_environment(&EnvironmentDescriptor::new(cube))
        .unwrap();
    renderer.render_frame().unwrap();
    renderer.render_frame().unwrap();
    let lines = create_pipeline_lines(&journal);
    // Deux formats de destination = deux permutations, créées UNE
    // fois (le cache tient sur les frames suivantes), en LessEqual.
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("create_pipeline chaos.sky.Rgba8UnormSrgb "));
    assert!(lines[1].starts_with("create_pipeline chaos.sky "));
    for line in &lines {
        assert!(line.ends_with(" depth=less_equal"));
    }
}

#[test]
fn the_environment_line_travels_with_the_plan() {
    let (mut renderer, journal) = mock_renderer();
    let cube = env_cubemap(&mut renderer, "sky");
    renderer
        .set_environment(&EnvironmentDescriptor::new(cube).with_intensity(2.0))
        .unwrap();
    renderer.set_exposure(1.5).unwrap();
    renderer.render_frame().unwrap();
    let last_environment = |journal: &Journal| {
        journal
            .entries()
            .iter()
            .rfind(|entry| entry.starts_with("environment "))
            .cloned()
    };
    assert_eq!(
        last_environment(&journal).as_deref(),
        Some("environment intensity=2 exposure=1.5")
    );

    // Sans environnement, l'exposition hors défaut voyage seule.
    renderer.clear_environment().unwrap();
    renderer.render_frame().unwrap();
    assert_eq!(
        last_environment(&journal).as_deref(),
        Some("environment intensity=0 exposure=1.5")
    );
}

#[test]
fn exposure_is_validated_and_persistent() {
    let (mut renderer, _journal) = mock_renderer();
    assert_eq!(renderer.exposure(), 1.0);
    for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let refused = renderer.set_exposure(invalid).unwrap_err();
        assert!(refused.to_string().contains("positive, finite"));
    }
    renderer.set_exposure(2.0).unwrap();
    renderer.clear_draws();
    assert_eq!(renderer.exposure(), 2.0);
}

#[test]
fn environment_info_reflects_the_state() {
    let (mut renderer, _journal) = mock_renderer();
    let cube = renderer
        .create_texture(
            &TextureDescriptor::cube("hdr", 2, TextureFormat::Rgba16Float, vec![0; 192])
                .with_mips(TextureMips::Generate),
        )
        .unwrap();
    renderer
        .set_environment(
            &EnvironmentDescriptor::new(cube)
                .with_intensity(0.8)
                .with_sky(false),
        )
        .unwrap();
    let info = renderer.environment_info().unwrap();
    assert_eq!(info.label, "hdr");
    assert_eq!(info.intensity, 0.8);
    assert!(!info.sky);
    assert_eq!(info.mip_levels, 2);
    renderer.clear_environment().unwrap();
    assert!(renderer.environment_info().is_none());
}

#[test]
fn the_environment_checkpoint() {
    let (mut renderer, journal) = mock_renderer();
    let target = small_target(&mut renderer, "viewport");
    renderer
        .add_pass(
            &RenderPassDescriptor::new("mirror", RenderDestination::Target(target)).with_order(-1),
        )
        .unwrap();
    let mesh = lit_quad_mesh(&mut renderer, "sphere");
    let mut materials = Vec::new();
    for (index, metallic) in [0.0, 1.0].iter().enumerate() {
        materials.push(
            renderer
                .create_material(
                    &MaterialDescriptor::new(format!("grid.{index}"), MaterialModel::Pbr)
                        .with_metallic(*metallic)
                        .with_roughness(0.2),
                )
                .unwrap(),
        );
    }
    // L'environnement HDR mippé : la rugosité IBL parcourt la chaîne.
    let hdr = renderer
        .create_texture(
            &TextureDescriptor::cube("env", 2, TextureFormat::Rgba16Float, vec![0; 192])
                .with_mips(TextureMips::Generate),
        )
        .unwrap();
    renderer
        .set_environment(&EnvironmentDescriptor::new(hdr))
        .unwrap();

    for material in &materials {
        renderer.queue_draw(DrawCommand {
            mesh,
            material: *material,
            transform: Transform::IDENTITY,
        });
    }
    renderer.render_frame().unwrap();

    // Le ciel couvre les deux passes ; métaux et diélectriques
    // partagent la permutation PBR sous le même environnement.
    let lines = render_lines(&journal);
    assert!(lines[0].contains(SKY_DRAW));
    assert!(lines[1].contains(SKY_DRAW));
    assert_eq!(lines[1].matches("m=[").count(), 3);
    assert_eq!(
        create_pipeline_lines(&journal)
            .iter()
            .filter(|line| line.contains("chaos.material.pbr"))
            .count(),
        1
    );
    assert_eq!(
        journal
            .entries()
            .iter()
            .rfind(|entry| entry.starts_with("environment "))
            .map(String::as_str),
        Some("environment intensity=1 exposure=1")
    );

    // L'exposition et l'intensité se règlent SANS rebind ni recréation.
    renderer.set_exposure(2.0).unwrap();
    renderer
        .set_environment(&EnvironmentDescriptor::new(hdr).with_intensity(0.5))
        .unwrap();
    let pipelines_before = create_pipeline_lines(&journal).len();
    renderer.render_frame().unwrap();
    assert_eq!(environment_lines(&journal).len(), 1);
    assert_eq!(create_pipeline_lines(&journal).len(), pipelines_before);
    assert_eq!(
        journal
            .entries()
            .iter()
            .rfind(|entry| entry.starts_with("environment "))
            .map(String::as_str),
        Some("environment intensity=0.5 exposure=2")
    );

    // Effacé : le ciel disparaît, le fond uni et l'ambiante restent.
    renderer.clear_environment().unwrap();
    renderer.render_frame().unwrap();
    assert!(!render_lines(&journal).pop().unwrap().contains(SKY_DRAW));
}
