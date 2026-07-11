//! Les diagnostics et la robustesse : capabilities, limites device,
//! budget CPU, compteurs de frame exacts et checkpoints V1.

use super::*;

#[test]
fn the_mock_reports_deterministic_capabilities() {
    let (renderer, journal) = mock_renderer();
    let capabilities = renderer.capabilities();
    assert_eq!(capabilities.backend, "mock");
    assert_eq!(capabilities.adapter, "journal");
    assert_eq!(
        capabilities.limits,
        crate::capabilities::DeviceLimits::default()
    );
    // AUCUNE feature optionnelle n'est supposée : les timestamps
    // sont COUPÉS avec leur raison — et rien n'empêche de rendre.
    assert!(matches!(
        &capabilities.decision("timestamp queries").unwrap().status,
        crate::capabilities::CapabilityStatus::Disabled { reason } if !reason.is_empty()
    ));
    // La consultation n'écrit RIEN au journal.
    assert!(journal.entries().is_empty());
}

#[test]
fn device_limits_refuse_oversized_resources_by_name() {
    let (mut renderer, _journal) = mock_renderer();
    // La texture au-delà de la limite device : refusée en NOMMANT
    // la valeur ET la limite — jamais une erreur backend.
    let refused = renderer
        .create_texture(&TextureDescriptor::sampled(
            "huge",
            8193,
            1,
            TextureFormat::R8Unorm,
            vec![0; 8193],
        ))
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("8193x1 exceeds the device texture limit (8192)")
    );
    let refused = renderer
        .create_render_target(&RenderTargetDescriptor::new(
            "huge",
            8193,
            1,
            TextureFormat::Rgba8UnormSrgb,
        ))
        .unwrap_err();
    assert!(refused.to_string().contains("device texture limit (8192)"));
    // À la limite EXACTE : accepté.
    assert!(
        renderer
            .create_texture(&TextureDescriptor::sampled(
                "edge",
                8192,
                1,
                TextureFormat::R8Unorm,
                vec![0; 8192],
            ))
            .is_ok()
    );
}

#[test]
fn lowered_device_limits_speak_for_the_device() {
    // Un device plus petit que l'engine : chaque refus parle au nom
    // du DEVICE — distinct des bornes engine.
    let limits = crate::capabilities::DeviceLimits {
        max_texture_2d: 1024,
        max_buffer_bytes: 1000,
        max_anisotropy: 8,
        ..crate::capabilities::DeviceLimits::default()
    };
    let (mut renderer, _journal) = mock_renderer_with_limits(limits);
    // Texture 2048 : légale pour l'engine, refusée par CE device.
    let refused = renderer
        .create_texture(&TextureDescriptor::sampled(
            "big",
            2048,
            1,
            TextureFormat::R8Unorm,
            vec![0; 2048],
        ))
        .unwrap_err();
    assert!(refused.to_string().contains("device texture limit (1024)"));
    // Buffer 1001 octets : refusé par les DEUX chemins — le public
    // et celui des meshes.
    let refused = renderer
        .create_buffer(&BufferDescriptor::vertex("big", vec![0; 1001]))
        .unwrap_err();
    assert!(refused.to_string().contains("device buffer limit (1000)"));
    let sphere = LitGeometry::sphere([0.0, 0.0, 0.0], 1.0, 16, 16);
    let refused = renderer.create_lit_mesh("ball", &sphere).unwrap_err();
    assert!(refused.to_string().contains("device buffer limit (1000)"));
    // L'ombre 2048 : le descripteur la VALIDE (16..=8192), le
    // device la refuse — le message nomme la borne DEVICE.
    let descriptor = DirectionalShadowDescriptor::new(ShadowVolume::new(
        Vec3::ZERO,
        Vec3::new(10.0, 10.0, 10.0),
    ));
    let refused = renderer.set_directional_shadow(&descriptor).unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("shadow map resolution 2048 exceeds the device texture limit (1024)")
    );
    assert!(
        renderer
            .set_directional_shadow(&descriptor.with_resolution(1024))
            .is_ok()
    );
    // L'anisotropie x16 : légale pour le descripteur (tout-Linear
    // respecté), refusée par CE device (plafond x8).
    let refused = renderer
        .create_sampler(
            &SamplerDescriptor::new("aniso")
                .with_mip_filter(SamplerFilter::Linear)
                .with_anisotropy(16),
        )
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("anisotropy x16 exceeds the device ceiling (x8)")
    );
    assert!(
        renderer
            .create_sampler(
                &SamplerDescriptor::new("aniso.ok")
                    .with_mip_filter(SamplerFilter::Linear)
                    .with_anisotropy(8),
            )
            .is_ok()
    );
    // À la limite exacte : accepté.
    assert!(
        renderer
            .create_texture(&TextureDescriptor::sampled(
                "edge",
                1024,
                1,
                TextureFormat::R8Unorm,
                vec![0; 1024],
            ))
            .is_ok()
    );
}

#[test]
fn checkpoint_robustness_v1_no_capability_is_implicit() {
    // LE checkpoint Robustesse V1 : le rapport COMPLET (chaque
    // domaine expliqué), les configurations impossibles refusées en
    // nommant, une feature optionnelle absente n'empêche RIEN, et
    // la consultation ne trouble jamais le journal.
    let (mut renderer, journal) = mock_renderer();
    let capabilities = renderer.capabilities().clone();
    assert!(!capabilities.backend.is_empty());
    assert!(!capabilities.adapter.is_empty());
    for decision in &capabilities.decisions {
        assert!(!decision.domain.is_empty());
        assert!(!decision.detail.is_empty());
        if let crate::capabilities::CapabilityStatus::Disabled { reason }
        | crate::capabilities::CapabilityStatus::Fallback { reason } = &decision.status
        {
            assert!(
                !reason.is_empty(),
                "{} must explain itself",
                decision.domain
            );
        }
    }
    // Le Display est la lecture sans UI.
    let text = capabilities.to_string();
    assert!(text.contains("capabilities: mock on journal"));
    assert!(text.contains("timestamp queries: disabled"));
    // Les timestamps COUPÉS n'empêchent rien : la frame rend, le
    // temps GPU est dit indisponible.
    let material = plain_material(&mut renderer, "prop");
    let mesh = renderer.create_mesh("cube", &cube()).unwrap();
    renderer.queue_draw(DrawCommand {
        mesh,
        material,
        transform: Transform::IDENTITY,
    });
    renderer.render_frame().unwrap();
    assert_eq!(renderer.diagnostics().frame.resolved, 1);
    assert!(matches!(
        &renderer.diagnostics().gpu,
        crate::diagnostics::GpuTiming::Unavailable { reason } if !reason.is_empty()
    ));
    // Une configuration IMPOSSIBLE échoue proprement, le registre
    // intact : la même scène rend encore.
    assert!(
        renderer
            .create_texture(&TextureDescriptor::sampled(
                "huge",
                9000,
                9000,
                TextureFormat::R8Unorm,
                vec![0; 81_000_000],
            ))
            .is_err()
    );
    renderer.render_frame().unwrap();
    assert_eq!(renderer.diagnostics().frame.resolved, 1);
    // Le journal n'a jamais vu passer ni le rapport ni le refus.
    assert!(
        journal
            .entries()
            .iter()
            .all(|entry| !entry.contains("huge") && !entry.contains("capabilities"))
    );
}

#[test]
fn the_mock_backend_declares_its_gpu_time_unavailable() {
    // « Aucune valeur ne doit être inventée » : un backend qui ne
    // mesure pas le DIT — la raison est nommée, jamais un zéro.
    let (renderer, _journal) = mock_renderer();
    assert!(matches!(
        renderer.backend.gpu_frame_time(),
        crate::diagnostics::GpuTiming::Unavailable { reason } if !reason.is_empty()
    ));
    // Avant la première frame : le snapshot par défaut, honnête.
    assert!(matches!(
        &renderer.diagnostics().gpu,
        crate::diagnostics::GpuTiming::Unavailable { reason } if !reason.is_empty()
    ));
}

#[test]
fn the_cpu_budget_is_validated_and_stored() {
    let (mut renderer, _journal) = mock_renderer();
    assert_eq!(renderer.diagnostics().budget.budget_ms, None);
    renderer.set_cpu_budget(Some(4.0));
    assert_eq!(renderer.diagnostics().budget.budget_ms, Some(4.0));
    renderer.set_cpu_budget(Some(f32::NAN));
    renderer.set_cpu_budget(Some(-1.0));
    renderer.set_cpu_budget(Some(0.0));
    assert_eq!(renderer.diagnostics().budget.budget_ms, Some(4.0));
    renderer.set_cpu_budget(None);
    assert_eq!(renderer.diagnostics().budget.budget_ms, None);
}

#[test]
fn diagnostics_count_the_instanced_crowd_exactly() {
    // La scène de l'instancing : 500 compatibles + 1 solitaire +
    // 3 transparents — chaque compteur du snapshot est EXACT.
    let (mut renderer, _journal) = mock_renderer();
    let crowd = lit_caster(&mut renderer, "crowd");
    let loner = lit_caster(&mut renderer, "loner");
    let glass = renderer
        .create_material(
            &MaterialDescriptor::new("glass", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
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
    renderer.render_frame().unwrap();
    let frame = renderer.diagnostics().frame;
    assert_eq!(frame.submitted, 504);
    assert_eq!(frame.resolved, 504);
    assert_eq!(frame.classic_draws, 4);
    assert_eq!(frame.instanced_draws, 1);
    assert_eq!(frame.instances, 500);
    assert_eq!(frame.culled, 0);
    assert_eq!(frame.injected, 0);
    // Le quad éclairé fait 6 indices = 2 triangles : la foule en
    // 1000, le solitaire 2, les verres 6.
    assert_eq!(frame.triangles, 1008);
    // Trois pipelines se succèdent (instancié → classique →
    // transparent), trois bindings distincts (les verres partagent).
    assert_eq!(frame.pipeline_switches, 3);
    assert_eq!(frame.material_switches, 3);
    assert_eq!(frame.passes_executed, 1);
    let pass = &renderer.diagnostics().passes[0];
    assert_eq!(pass.draw_calls, 5);
    assert_eq!(pass.instances, 500);
    assert!(pass.resolve_cpu_ms.is_finite() && pass.resolve_cpu_ms >= 0.0);
    // Le coût CPU est MESURÉ, le GPU du mock est indisponible DIT.
    let cpu = renderer.diagnostics().cpu;
    assert!(cpu.total_ms >= cpu.backend_ms);
    assert!(matches!(
        &renderer.diagnostics().gpu,
        crate::diagnostics::GpuTiming::Unavailable { reason }
            if reason.contains("mock")
    ));
}

#[test]
fn diagnostics_measure_the_culling_gains() {
    let (mut renderer, _journal) = mock_renderer();
    let material = plain_material(&mut renderer, "crowd");
    let mesh = renderer.create_mesh("cube", &cube()).unwrap();
    for index in 0..10u32 {
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::from_translation(Vec3::new(250.0 * index as f32, 0.0, 0.0)),
        });
    }
    renderer.render_frame().unwrap();
    // Cinq visibles fusionnés, cinq cullés : le gain se LIT — et
    // les triangles ne comptent QUE les visibles (12 par cube).
    let frame = renderer.diagnostics().frame;
    assert_eq!(frame.submitted, 10);
    assert_eq!(frame.resolved, 5);
    assert_eq!(frame.culled, 5);
    assert_eq!(frame.instanced_draws, 1);
    assert_eq!(frame.instances, 5);
    assert_eq!(frame.triangles, 60);
}

#[test]
fn the_sky_and_debug_are_counted_honestly() {
    let (mut renderer, _journal) = mock_renderer();
    let cubemap = env_cubemap(&mut renderer, "diag.sky");
    renderer
        .set_environment(&EnvironmentDescriptor::new(cubemap))
        .unwrap();
    renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X));
    renderer.queue_debug(DebugDraw::point(Vec3::ZERO, 0.5).overlay());
    renderer.render_frame().unwrap();
    let frame = renderer.diagnostics().frame;
    // Le ciel = 1 injecté et 1 TRIANGLE ; le debug = 2 injectés et
    // 4 SEGMENTS (la ligne + la croix à trois axes) — jamais des
    // triangles.
    assert_eq!(frame.resolved, 3);
    assert_eq!(frame.injected, 3);
    assert_eq!(frame.triangles, 1);
    assert_eq!(frame.debug_segments, 4);
    assert_eq!(frame.classic_draws, 1);
    assert_eq!(frame.pipeline_switches, 3);
    assert_eq!(frame.material_switches, 0);
}

#[test]
fn surface_events_accumulate_by_reason() {
    for (outcome, check) in [
        (
            FrameOutcome::Skipped(FrameSkipReason::SurfaceUnavailable),
            0usize,
        ),
        (
            FrameOutcome::Skipped(FrameSkipReason::SurfaceReconfigured),
            1,
        ),
        (FrameOutcome::Skipped(FrameSkipReason::ZeroArea), 2),
        (FrameOutcome::Rendered, 3),
    ] {
        let (mut renderer, _journal) = mock_renderer_with(outcome);
        renderer.render_frame().unwrap();
        renderer.render_frame().unwrap();
        let surface = renderer.diagnostics().surface;
        let counters = [
            surface.skipped_unavailable,
            surface.reconfigured,
            surface.zero_area,
            surface.presented,
        ];
        for (index, counter) in counters.iter().enumerate() {
            assert_eq!(*counter, if index == check { 2 } else { 0 });
        }
    }
}

#[test]
fn the_budget_counts_overruns() {
    let (mut renderer, _journal) = mock_renderer();
    // Sans budget : jamais de dépassement.
    renderer.render_frame().unwrap();
    assert_eq!(renderer.diagnostics().budget.over_budget_frames, 0);
    assert!(!renderer.diagnostics().budget.last_frame_over);
    // Un budget minuscule : chaque frame dépasse, le cumul avance.
    renderer.set_cpu_budget(Some(f32::MIN_POSITIVE));
    renderer.render_frame().unwrap();
    renderer.render_frame().unwrap();
    assert_eq!(renderer.diagnostics().budget.over_budget_frames, 2);
    assert!(renderer.diagnostics().budget.last_frame_over);
    // Le budget retiré : le cumul reste, le présent redevient sain.
    renderer.set_cpu_budget(None);
    renderer.render_frame().unwrap();
    assert_eq!(renderer.diagnostics().budget.over_budget_frames, 2);
    assert!(!renderer.diagnostics().budget.last_frame_over);
}

#[test]
fn degraded_permutations_and_builtins_are_visible_fallbacks() {
    let (mut renderer, _journal) = mock_renderer();
    // Un material Unlit sans texture consomme les fallbacks builtin.
    let material = renderer
        .create_material(&MaterialDescriptor::new("bare", MaterialModel::Unlit))
        .unwrap();
    let mesh = renderer
        .create_textured_mesh(
            "pane",
            &TexturedGeometry::quad([0.0, 0.0, 0.0], 1.0, 1.0, 1.0),
        )
        .unwrap();
    renderer.queue_draw(DrawCommand {
        mesh,
        material,
        transform: Transform::IDENTITY,
    });
    // Une permutation d'ombre IMPOSSIBLE (layout sans position) est
    // mémoïsée — un chemin dégradé VISIBLE au snapshot.
    let layout = VertexLayout::packed(&[VertexAttributeFormat::Float32x2]);
    {
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
    }
    renderer.render_frame().unwrap();
    let fallbacks = renderer.diagnostics().fallbacks;
    assert_eq!(fallbacks.degraded_permutations, 1);
    assert!(fallbacks.fallback_textures >= 1);
    assert!(fallbacks.fallback_samplers >= 1);
}

#[test]
fn checkpoint_diagnostics_v1_the_frame_explains_itself() {
    // LE checkpoint Diagnostics V1 : la scène composée — foule
    // instanciée, hors-champ cullés, masked, transparents, ciel,
    // ombre, debug sous les deux profondeurs — et CHAQUE champ du
    // snapshot exact, stable sur deux frames, les gains de
    // l'instancing et du culling MESURABLES, le GPU indisponible
    // DIT, les ressources cohérentes.
    let (mut renderer, _journal) = mock_renderer();
    renderer
        .set_directional_shadow(&DirectionalShadowDescriptor::new(ShadowVolume::new(
            Vec3::new(100.0, 0.0, 0.0),
            Vec3::new(150.0, 20.0, 150.0),
        )))
        .unwrap();
    let cubemap = env_cubemap(&mut renderer, "chk.sky");
    renderer
        .set_environment(&EnvironmentDescriptor::new(cubemap))
        .unwrap();
    let crowd = lit_caster(&mut renderer, "crowd");
    let grid = renderer
        .create_material(
            &MaterialDescriptor::new("grid", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Masked),
        )
        .unwrap();
    let glass = renderer
        .create_material(
            &MaterialDescriptor::new("glass", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
    let queue_scene = |renderer: &mut Renderer| {
        renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 0.9));
        for index in 0..200u32 {
            renderer.queue_draw(DrawCommand {
                transform: Transform::from_translation(Vec3::new(index as f32, 0.0, 0.0)),
                ..crowd
            });
        }
        for index in 0..5u32 {
            renderer.queue_draw(DrawCommand {
                transform: Transform::from_translation(Vec3::new(5000.0 + index as f32, 0.0, 0.0)),
                ..crowd
            });
        }
        renderer.queue_draw(DrawCommand {
            mesh: crowd.mesh,
            material: grid,
            transform: Transform::IDENTITY,
        });
        for z in [-1.0, -2.0] {
            renderer.queue_draw(DrawCommand {
                mesh: crowd.mesh,
                material: glass,
                transform: Transform::from_translation(Vec3::new(0.0, 0.0, z)),
            });
        }
        renderer.queue_debug(DebugDraw::line(Vec3::ZERO, Vec3::X));
        renderer.queue_debug(DebugDraw::point(Vec3::ZERO, 0.5).overlay());
    };

    queue_scene(&mut renderer);
    renderer.render_frame().unwrap();
    let first = renderer.diagnostics().clone();
    let frame = first.frame;
    // Les gains MESURABLES : 208 soumis, 200 fusionnés en UN batch,
    // 5 cullés — draws ≫ draw_calls, en chiffres.
    assert_eq!(frame.submitted, 208);
    assert_eq!(frame.resolved, 206);
    assert_eq!(frame.classic_draws, 4);
    assert_eq!(frame.instanced_draws, 1);
    assert_eq!(frame.instances, 200);
    assert_eq!(frame.culled, 5);
    assert_eq!(frame.injected, 3);
    // Les triangles : la foule 400, le masked 2, les verres 4, le
    // ciel 1 — les segments de debug comptés À PART.
    assert_eq!(frame.triangles, 407);
    assert_eq!(frame.debug_segments, 4);
    // Six pipelines se succèdent (instancié → masked → ciel →
    // transparent → debug → overlay), trois bindings distincts.
    assert_eq!(frame.pipeline_switches, 6);
    assert_eq!(frame.material_switches, 3);
    assert_eq!(frame.passes_executed, 1);
    assert_eq!(frame.passes_skipped, 0);
    // L'ombre : 201 casters (la foule + le masked — les verres
    // jamais), 5 tentatives hors volume, 2 soumissions.
    assert_eq!(
        first.shadow,
        Some(crate::diagnostics::ShadowStats {
            draws: 201,
            draw_calls: 2,
            culled: 5,
            instances: 200,
            triangles: 402,
        })
    );
    // Les coûts CPU sont MESURÉS et cohérents, le GPU du mock est
    // indisponible AVEC sa raison, les ressources sont LA photo.
    assert!(first.cpu.total_ms.is_finite() && first.cpu.total_ms >= first.cpu.backend_ms);
    assert!(matches!(
        &first.gpu,
        crate::diagnostics::GpuTiming::Unavailable { reason } if reason.contains("mock")
    ));
    assert_eq!(first.resources, renderer.resource_stats());
    assert!(first.fallbacks.fallback_textures >= 1);
    assert_eq!(first.fallbacks.degraded_permutations, 0);
    assert_eq!(first.surface.presented, 1);
    assert_eq!(first.budget.over_budget_frames, 0);
    // Le Display porte les chiffres — utilisable sans UI.
    let text = first.to_string();
    assert!(text.contains("208 submitted -> 206 resolved"));
    assert!(text.contains("gpu: unavailable"));

    // FRAME 2 : la même scène — les COMPTEURS identiques (seuls les
    // temps varient), les cumulatifs avancent.
    renderer.clear_draws();
    queue_scene(&mut renderer);
    renderer.render_frame().unwrap();
    let second = renderer.diagnostics();
    assert_eq!(second.frame, first.frame);
    assert_eq!(second.shadow, first.shadow);
    assert_eq!(second.passes.len(), first.passes.len());
    assert_eq!(second.passes[0].draw_calls, first.passes[0].draw_calls);
    assert_eq!(second.surface.presented, 2);
}

#[test]
fn render_to_target_leaves_the_diagnostics_alone() {
    let (mut renderer, _journal) = mock_renderer();
    let material = plain_material(&mut renderer, "p");
    let mesh = renderer.create_mesh("cube", &cube()).unwrap();
    renderer.queue_draw(DrawCommand {
        mesh,
        material,
        transform: Transform::IDENTITY,
    });
    renderer.render_frame().unwrap();
    let before = renderer.diagnostics().clone();
    let target = small_target(&mut renderer, "viewport");
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
    assert_eq!(renderer.diagnostics(), &before);
}

#[test]
fn the_empty_plan_still_snapshots() {
    let (mut renderer, _journal) = mock_renderer();
    renderer
        .set_pass_enabled(renderer.main_pass(), false)
        .unwrap();
    renderer.render_frame().unwrap();
    let diagnostics = renderer.diagnostics();
    assert_eq!(diagnostics.frame.passes_executed, 0);
    assert_eq!(diagnostics.frame.passes_skipped, 1);
    assert_eq!(diagnostics.frame.resolved, 0);
    assert!(diagnostics.cpu.total_ms.is_finite());
    // Rien n'est parti au backend : aucun événement de surface.
    assert_eq!(diagnostics.surface.presented, 0);
}
