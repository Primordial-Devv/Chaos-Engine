//! Les ombres directionnelles : configuration validée, moisson des
//! casters, permutations depth-only, flags des materials et checkpoint V1.

use super::*;

#[test]
fn receive_shadows_off_is_refused_on_unlit_models() {
    let (mut renderer, journal) = mock_renderer();
    let vertex_color = renderer
        .create_material(
            &MaterialDescriptor::new("m", MaterialModel::VertexColor).without_shadow_receive(),
        )
        .unwrap_err();
    assert!(
        vertex_color
            .to_string()
            .contains("does not react to lighting")
    );
    assert!(vertex_color.to_string().contains("'receive_shadows'"));
    let unlit = renderer
        .create_material(
            &MaterialDescriptor::new("m", MaterialModel::Unlit).without_shadow_receive(),
        )
        .unwrap_err();
    assert!(unlit.to_string().contains("does not react to lighting"));

    // Accepté sur les modèles éclairés — et le binding le journalise
    // hors défaut. cast_shadows n'a pas de contrainte de modèle (une
    // géométrie projette quel que soit son shader).
    renderer
        .create_material(&MaterialDescriptor::new("l", MaterialModel::Lit).without_shadow_receive())
        .unwrap();
    assert!(binding_lines(&journal).pop().unwrap().contains("recv=off"));
    renderer
        .create_material(
            &MaterialDescriptor::new("v", MaterialModel::VertexColor).without_shadow_cast(),
        )
        .unwrap();
}

#[test]
fn material_info_reflects_the_shadow_flags() {
    let (mut renderer, _journal) = mock_renderer();
    let material = renderer
        .create_material(
            &MaterialDescriptor::new("m", MaterialModel::Pbr)
                .without_shadow_cast()
                .without_shadow_receive(),
        )
        .unwrap();
    let info = renderer.material_info(material).unwrap();
    assert!(!info.cast_shadows);
    assert!(!info.receive_shadows);
    let lit = renderer
        .create_material(&MaterialDescriptor::new("d", MaterialModel::Lit))
        .unwrap();
    let info = renderer.material_info(lit).unwrap();
    assert!(info.cast_shadows);
    assert!(info.receive_shadows);
}

#[test]
fn configuring_the_shadow_reaches_the_backend_once() {
    let (mut renderer, journal) = mock_renderer();
    renderer.set_directional_shadow(&demo_shadow()).unwrap();
    assert_eq!(
        set_shadow_lines(&journal),
        vec!["set_shadow resolution=2048"]
    );

    // Re-poser la MÊME résolution (biais retouchés) = zéro appel
    // backend — les biais sont des données par frame.
    renderer
        .set_directional_shadow(&demo_shadow().with_depth_bias(0.01).with_normal_bias(0.1))
        .unwrap();
    assert_eq!(set_shadow_lines(&journal).len(), 1);

    // Une autre résolution recrée la map.
    renderer
        .set_directional_shadow(&demo_shadow().with_resolution(1024))
        .unwrap();
    assert_eq!(
        set_shadow_lines(&journal),
        vec!["set_shadow resolution=2048", "set_shadow resolution=1024"]
    );

    // Effacer libère — et l'effacement répété est un no-op.
    renderer.clear_directional_shadow().unwrap();
    renderer.clear_directional_shadow().unwrap();
    assert_eq!(
        set_shadow_lines(&journal),
        vec![
            "set_shadow resolution=2048",
            "set_shadow resolution=1024",
            "set_shadow none"
        ]
    );
}

#[test]
fn shadow_settings_are_validated_before_the_backend() {
    let (mut renderer, journal) = mock_renderer();
    let refused = renderer
        .set_directional_shadow(&demo_shadow().with_resolution(8))
        .unwrap_err();
    assert!(refused.to_string().contains("16..=8192"));
    let flat = renderer
        .set_directional_shadow(&DirectionalShadowDescriptor::new(ShadowVolume::new(
            Vec3::ZERO,
            Vec3::new(10.0, 0.0, 10.0),
        )))
        .unwrap_err();
    assert!(flat.to_string().contains("half extents"));
    assert!(set_shadow_lines(&journal).is_empty());
    assert!(renderer.directional_shadow_info().is_none());
}

#[test]
fn shadow_info_mirrors_the_state() {
    let (mut renderer, _journal) = mock_renderer();
    assert!(renderer.directional_shadow_info().is_none());
    renderer
        .set_directional_shadow(&demo_shadow().with_resolution(512).with_depth_bias(0.01))
        .unwrap();
    let info = renderer.directional_shadow_info().unwrap();
    assert_eq!(info.resolution, 512);
    assert_eq!(info.depth_bias, 0.01);
    assert_eq!(info.volume.half_extents, Vec3::new(10.0, 10.0, 10.0));
    renderer.clear_directional_shadow().unwrap();
    assert!(renderer.directional_shadow_info().is_none());
}

#[test]
fn the_shadow_pass_travels_first_with_the_casting_draws() {
    let (mut renderer, journal) = mock_renderer();
    renderer.set_directional_shadow(&demo_shadow()).unwrap();
    renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
    let command = lit_caster(&mut renderer, "caster");
    renderer.queue_draw(command);
    renderer.render_frame().unwrap();

    // Le pipeline d'ombre est une permutation depth-only dédiée,
    // étiquetée par le stride de son layout (LitVertex = 32).
    assert!(
        create_pipeline_lines(&journal)
            .iter()
            .any(|line| line.starts_with("create_pipeline chaos.shadow.32 ")
                && line.contains(" depth_only"))
    );
    // La ligne shadow PRÉCÈDE la ligne render de la passe principale,
    // porte le caster (binding None) et la résolution.
    let entries = journal.entries();
    let shadow_at = entries
        .iter()
        .position(|entry| entry.starts_with("shadow "))
        .unwrap();
    let render_at = entries
        .iter()
        .position(|entry| entry.starts_with("render "))
        .unwrap();
    assert!(shadow_at < render_at);
    let shadow = &entries[shadow_at];
    assert!(shadow.contains("res=2048"));
    assert!(shadow.contains("light=0"));
    assert!(shadow.contains("b=None"));
    // Le rapport dédié reflète la passe.
    assert_eq!(
        renderer.frame_report().shadow,
        Some(ShadowReport {
            draws: 1,
            draw_calls: 1,
            culled: 0,
            resolution: 2048
        })
    );
}

#[test]
fn without_a_directional_light_the_shadow_pass_is_absent() {
    let (mut renderer, journal) = mock_renderer();
    renderer.set_directional_shadow(&demo_shadow()).unwrap();
    let command = lit_caster(&mut renderer, "caster");
    renderer.queue_draw(command);

    // Aucune lumière : pas de passe d'ombre, rien de fatal.
    renderer.render_frame().unwrap();
    assert!(shadow_lines(&journal).is_empty());
    assert_eq!(renderer.frame_report().shadow, None);

    // Une ponctuelle seule ne projette pas en V1.
    renderer.submit_light(Light::point(Vec3::Y, Color::WHITE, 1.0, 5.0));
    renderer.render_frame().unwrap();
    assert!(shadow_lines(&journal).is_empty());

    // Une directionnelle DÉSACTIVÉE est filtrée de la collection —
    // elle ne projette pas non plus.
    let mut sun = Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0);
    sun.set_enabled(false);
    renderer.submit_light(sun);
    renderer.render_frame().unwrap();
    assert!(shadow_lines(&journal).is_empty());
    assert_eq!(renderer.frame_report().shadow, None);
}

#[test]
fn the_first_enabled_directional_is_the_caster() {
    let (mut renderer, journal) = mock_renderer();
    renderer.set_directional_shadow(&demo_shadow()).unwrap();
    let command = lit_caster(&mut renderer, "caster");
    renderer.queue_draw(command);
    // Une ponctuelle soumise AVANT : la directionnelle est à
    // l'index 1 de la collection — l'index voyage au shader.
    renderer.submit_light(Light::point(Vec3::Y, Color::WHITE, 1.0, 5.0));
    renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
    renderer.submit_light(Light::directional(Vec3::NEG_X, Color::WHITE, 0.5));
    renderer.render_frame().unwrap();
    let shadow = shadow_lines(&journal).pop().unwrap();
    assert!(shadow.contains("light=1"));
}

#[test]
fn transparent_and_non_casting_materials_never_cast() {
    let (mut renderer, journal) = mock_renderer();
    renderer.set_directional_shadow(&demo_shadow()).unwrap();
    renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
    let caster = lit_caster(&mut renderer, "caster");
    renderer.queue_draw(caster);
    let opted_out = renderer
        .create_material(&MaterialDescriptor::new("shy", MaterialModel::Lit).without_shadow_cast())
        .unwrap();
    let glass = renderer
        .create_material(
            &MaterialDescriptor::new("glass", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
    let mesh = lit_quad_mesh(&mut renderer, "extra");
    renderer.queue_draw(DrawCommand {
        mesh,
        material: opted_out,
        transform: Transform::IDENTITY,
    });
    renderer.queue_draw(DrawCommand {
        mesh,
        material: glass,
        transform: Transform::IDENTITY,
    });
    renderer.render_frame().unwrap();
    // Trois draws dans la passe, UN SEUL caster dans l'ombre.
    assert_eq!(
        renderer.frame_report().shadow,
        Some(ShadowReport {
            draws: 1,
            draw_calls: 1,
            culled: 0,
            resolution: 2048
        })
    );
    let shadow = shadow_lines(&journal).pop().unwrap();
    assert_eq!(shadow.matches("m=[").count(), 1);
}

#[test]
fn the_sky_never_casts_a_shadow() {
    let (mut renderer, journal) = mock_renderer();
    let cubemap = renderer
        .create_texture(&TextureDescriptor::cube(
            "sky.cube",
            1,
            TextureFormat::Rgba8Unorm,
            vec![0; 4 * 6],
        ))
        .unwrap();
    renderer
        .set_environment(&EnvironmentDescriptor::new(cubemap))
        .unwrap();
    renderer.set_directional_shadow(&demo_shadow()).unwrap();
    renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
    let command = lit_caster(&mut renderer, "caster");
    renderer.queue_draw(command);
    renderer.render_frame().unwrap();
    // La passe principale porte le caster ET le ciel injecté ; la
    // passe d'ombre ne porte QUE le caster.
    assert_eq!(
        renderer.frame_report().shadow,
        Some(ShadowReport {
            draws: 1,
            draw_calls: 1,
            culled: 0,
            resolution: 2048
        })
    );
    let render = render_lines(&journal).pop().unwrap();
    assert_eq!(render.matches("m=[").count(), 2);
}

#[test]
fn shadow_pipelines_are_permutations_cached_by_layout_and_state() {
    let (mut renderer, journal) = mock_renderer();
    renderer.set_directional_shadow(&demo_shadow()).unwrap();
    renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
    let first = lit_caster(&mut renderer, "first");
    let second = lit_caster(&mut renderer, "second");
    let leafy = renderer
        .create_material(&MaterialDescriptor::new("leafy", MaterialModel::Lit).double_sided())
        .unwrap();
    renderer.queue_draw(first);
    renderer.queue_draw(second);
    renderer.queue_draw(DrawCommand {
        mesh: first.mesh,
        material: leafy,
        transform: Transform::IDENTITY,
    });
    renderer.render_frame().unwrap();
    renderer.queue_draw(first);
    renderer.render_frame().unwrap();
    // UNE permutation par (layout, culling, instanced) — stable
    // entre frames : deux materials Lit partagent la même, le
    // double-sided a la sienne, et le DOUBLON de `first` (la file
    // persiste entre les frames) fusionne en caster instancié à la
    // deuxième frame — sa permutation `.instanced` est la troisième
    // et dernière. Le label porte le stride du layout (LitVertex =
    // 32).
    let shadow_pipelines: Vec<String> = create_pipeline_lines(&journal)
        .into_iter()
        .filter(|line| line.contains("chaos.shadow"))
        .collect();
    assert_eq!(shadow_pipelines.len(), 3);
    assert!(shadow_pipelines[0].starts_with("create_pipeline chaos.shadow.32 "));
    assert!(shadow_pipelines[1].starts_with("create_pipeline chaos.shadow.32.double_sided "));
    assert!(shadow_pipelines[2].starts_with("create_pipeline chaos.shadow.32.instanced "));
    assert!(shadow_lines(&journal).pop().unwrap().contains("inst=2"));
}

#[test]
fn a_layout_without_position_is_excluded_from_casting() {
    let (mut renderer, journal) = mock_renderer();
    let layout = VertexLayout::packed(&[VertexAttributeFormat::Float32x2]);
    let mut context = PipelineContext {
        pipeline_cache: &mut renderer.pipeline_cache,
        sky_pipelines: &mut renderer.sky_pipelines,
        shadow_pipelines: &mut renderer.shadow_pipelines,
        instanced_pipelines: &mut renderer.instanced_pipelines,
        debug_pipelines: &mut renderer.debug_pipelines,
        backend: renderer.backend.as_mut(),
        shaders: &renderer.shaders,
        lifetime: &mut renderer.lifetime,
    };
    assert!(Renderer::resolve_shadow_pipeline(&mut context, &layout, false, false).is_none());
    // Mémoïsé : le second appel ne retente rien, aucun pipeline créé.
    assert!(Renderer::resolve_shadow_pipeline(&mut context, &layout, false, false).is_none());
    assert!(
        create_pipeline_lines(&journal)
            .iter()
            .all(|line| !line.contains("chaos.shadow"))
    );
}

#[test]
fn render_to_target_carries_no_shadow_pass() {
    let (mut renderer, journal) = mock_renderer();
    renderer.set_directional_shadow(&demo_shadow()).unwrap();
    renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
    let target = renderer
        .create_render_target(&RenderTargetDescriptor::new(
            "vignette",
            64,
            64,
            TextureFormat::Rgba8UnormSrgb,
        ))
        .unwrap();
    let command = lit_caster(&mut renderer, "caster");
    renderer
        .render_to_target(target, Color::BLACK, Mat4::IDENTITY, &[command])
        .unwrap();
    // Le chemin immédiat ne rend pas d'ombre (il échantillonne la
    // map du dernier plan) et ne touche pas le rapport.
    assert!(shadow_lines(&journal).is_empty());
    assert_eq!(renderer.frame_report().shadow, None);
}

#[test]
fn an_empty_plan_skips_the_shadow_too() {
    let (mut renderer, journal) = mock_renderer();
    renderer.set_directional_shadow(&demo_shadow()).unwrap();
    renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
    let main = renderer.main_pass();
    renderer.set_pass_enabled(main, false).unwrap();
    renderer.render_frame().unwrap();
    assert!(shadow_lines(&journal).is_empty());
    assert_eq!(renderer.frame_report().shadow, None);
}

#[test]
fn shadow_maps_are_counted_in_the_stats() {
    let (mut renderer, _journal) = mock_renderer();
    let baseline = renderer.resource_stats();
    assert_eq!(baseline.shadow_maps, KindStats::default());
    renderer
        .set_directional_shadow(&demo_shadow().with_resolution(1024))
        .unwrap();
    let stats = renderer.resource_stats();
    assert_eq!(stats.shadow_maps.alive, 1);
    assert_eq!(stats.shadow_maps.bytes, 1024 * 1024 * 4);
    assert_eq!(
        stats.estimated_bytes,
        baseline.estimated_bytes + 1024 * 1024 * 4
    );
    renderer.clear_directional_shadow().unwrap();
    assert_eq!(renderer.resource_stats(), baseline);
}

#[test]
fn checkpoint_shadows_v1_full_scene_over_two_frames() {
    // LE checkpoint Shadows V1 : une scène complète (caster,
    // non-caster, transparent, receive-off) sous réglages d'ombre,
    // le volume retouché entre deux frames SANS recréation backend,
    // le toggle du soleil observable, l'effacement revenant au
    // niveau de base.
    let (mut renderer, journal) = mock_renderer();
    renderer.set_directional_shadow(&demo_shadow()).unwrap();
    renderer.set_ambient_light(Color::WHITE, 0.05);

    let caster = lit_caster(&mut renderer, "chk.caster");
    let shy = renderer
        .create_material(
            &MaterialDescriptor::new("chk.shy", MaterialModel::Lit).without_shadow_cast(),
        )
        .unwrap();
    let numb = renderer
        .create_material(
            &MaterialDescriptor::new("chk.numb", MaterialModel::Pbr).without_shadow_receive(),
        )
        .unwrap();
    let glass = renderer
        .create_material(
            &MaterialDescriptor::new("chk.glass", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
    let mesh = lit_quad_mesh(&mut renderer, "chk.mesh");
    let mut sun = Light::directional(Vec3::new(-1.0, -2.0, -1.0), Color::WHITE, 0.9);

    // FRAME 1 : quatre draws, DEUX casters (le caster + le numb —
    // receive-off projette quand même), la passe d'ombre en tête.
    renderer.submit_light(sun.clone());
    renderer.queue_draw(caster);
    for material in [shy, numb, glass] {
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
    }
    renderer.render_frame().unwrap();
    assert_eq!(
        renderer.frame_report().shadow,
        Some(ShadowReport {
            draws: 2,
            draw_calls: 2,
            culled: 0,
            resolution: 2048
        })
    );
    let first_shadow = shadow_lines(&journal).pop().unwrap();

    // FRAME 2 : le volume et les biais retouchés À CHAUD — zéro
    // set_shadow backend de plus, la vue de lumière change.
    renderer
        .set_directional_shadow(
            &DirectionalShadowDescriptor::new(ShadowVolume::new(
                Vec3::new(2.0, 0.0, 2.0),
                Vec3::new(6.0, 6.0, 6.0),
            ))
            .with_depth_bias(0.004),
        )
        .unwrap();
    renderer.clear_draws();
    renderer.submit_light(sun.clone());
    renderer.queue_draw(caster);
    renderer.render_frame().unwrap();
    assert_eq!(set_shadow_lines(&journal).len(), 1);
    let second_shadow = shadow_lines(&journal).pop().unwrap();
    assert_ne!(first_shadow, second_shadow);
    assert_eq!(
        renderer.frame_report().shadow,
        Some(ShadowReport {
            draws: 1,
            draw_calls: 1,
            culled: 0,
            resolution: 2048
        })
    );

    // FRAME 3 : le soleil coupé — la passe d'ombre disparaît, la
    // scène continue.
    sun.set_enabled(false);
    renderer.clear_draws();
    renderer.submit_light(sun);
    renderer.queue_draw(caster);
    renderer.render_frame().unwrap();
    assert_eq!(shadow_lines(&journal).len(), 2);
    assert_eq!(renderer.frame_report().shadow, None);

    // Effacement : la map libérée, le rapport et les stats au
    // niveau de base.
    renderer.clear_directional_shadow().unwrap();
    assert_eq!(
        set_shadow_lines(&journal).last().map(String::as_str),
        Some("set_shadow none")
    );
    assert_eq!(renderer.resource_stats().shadow_maps, KindStats::default());
    assert!(renderer.directional_shadow_info().is_none());
}
