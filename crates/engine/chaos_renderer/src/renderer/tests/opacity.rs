//! Les catégories d'opacité : contrats masked, tri des transparents,
//! l'ordre opaque → masked → ciel → transparent et le checkpoint V1.

use super::*;

#[test]
fn opaque_draws_come_before_transparent_ones() {
    let (mut renderer, journal) = mock_renderer();
    let glass = renderer
        .create_material(
            &MaterialDescriptor::new("glass", MaterialModel::VertexColor)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
    let solid = plain_material(&mut renderer, "solid");
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    // Le transparent est SOUMIS d'abord — le plan le rend en dernier.
    renderer.queue_draw(DrawCommand {
        mesh,
        material: glass,
        transform: Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
    });
    renderer.queue_draw(DrawCommand {
        mesh,
        material: solid,
        transform: Transform::from_translation(Vec3::new(2.0, 0.0, 0.0)),
    });
    renderer.render_frame().unwrap();
    let line = render_lines(&journal).pop().unwrap();
    let opaque_at = line.find("m=[2, 0, 0]").unwrap();
    let transparent_at = line.find("m=[1, 0, 0]").unwrap();
    assert!(opaque_at < transparent_at);

    // La même partition s'applique au rendu immédiat vers une cible
    // — avec la caméra large : la vignette cull avec SA vue.
    let target = small_target(&mut renderer, "viewport");
    renderer
        .render_to_target(
            target,
            Color::BLACK,
            Mat4::from_scale(Vec3::new(0.001, 0.001, -0.001)),
            &[
                DrawCommand {
                    mesh,
                    material: glass,
                    transform: Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
                },
                DrawCommand {
                    mesh,
                    material: solid,
                    transform: Transform::from_translation(Vec3::new(2.0, 0.0, 0.0)),
                },
            ],
        )
        .unwrap();
    let offscreen = render_lines(&journal).pop().unwrap();
    let opaque_at = offscreen.find("m=[2, 0, 0]").unwrap();
    let transparent_at = offscreen.find("m=[1, 0, 0]").unwrap();
    assert!(opaque_at < transparent_at);
}

#[test]
fn masked_contracts_are_validated_at_creation() {
    let (mut renderer, _journal) = mock_renderer();
    // Masked sans entrées material : aucun alpha à tester — refusé
    // en nommant la règle.
    let blind = renderer
        .create_material(
            &MaterialDescriptor::new("m", MaterialModel::VertexColor)
                .with_opacity(MaterialOpacity::Masked),
        )
        .unwrap_err();
    assert!(blind.to_string().contains("no alpha to test"));
    // Le cutoff hors défaut est réservé à Masked.
    let inert = renderer
        .create_material(&MaterialDescriptor::new("m", MaterialModel::Lit).with_alpha_cutoff(0.3))
        .unwrap_err();
    assert!(inert.to_string().contains("'alpha_cutoff'"));
    // Les bornes du cutoff sont nommées.
    let wild = renderer
        .create_material(
            &MaterialDescriptor::new("m", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Masked)
                .with_alpha_cutoff(1.5),
        )
        .unwrap_err();
    assert!(wild.to_string().contains("0..=1"));
    // Accepté sur les modèles à entrées — Unlit, Lit, Pbr et le
    // Custom délégué (son shader doit exposer fs_masked).
    renderer
        .create_material(
            &MaterialDescriptor::new("grid", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Masked)
                .with_alpha_cutoff(0.35),
        )
        .unwrap();
    let probe = renderer
        .create_material(
            &MaterialDescriptor::new("probe", MaterialModel::Unlit)
                .with_opacity(MaterialOpacity::Masked),
        )
        .unwrap();
    let info = renderer.material_info(probe).unwrap();
    assert_eq!(info.opacity, MaterialOpacity::Masked);
    assert_eq!(info.alpha_cutoff, 0.5);
}

#[test]
fn the_masked_permutation_has_its_own_pipeline_and_entry() {
    let (mut renderer, journal) = mock_renderer();
    renderer
        .create_material(&MaterialDescriptor::new("solid", MaterialModel::Lit))
        .unwrap();
    renderer
        .create_material(
            &MaterialDescriptor::new("grid", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Masked),
        )
        .unwrap();
    // Même modèle, opacités différentes : DEUX permutations — la
    // masked porte son suffixe de label et son entrée fs_masked.
    let lines = create_pipeline_lines(&journal);
    assert!(lines[0].starts_with("create_pipeline chaos.material.lit "));
    assert!(!lines[0].contains(" entry="));
    assert!(lines[1].starts_with("create_pipeline chaos.material.lit.masked "));
    assert!(lines[1].contains(" entry=fs_masked"));
    assert!(!lines[1].contains(" blend=alpha"));
    // Un second material masked du même modèle réutilise la
    // permutation (le cache déduplique).
    renderer
        .create_material(
            &MaterialDescriptor::new("fence", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Masked),
        )
        .unwrap();
    assert_eq!(create_pipeline_lines(&journal).len(), 2);
}

#[test]
fn the_alpha_cutoff_updates_in_place() {
    let (mut renderer, journal) = mock_renderer();
    let grid = renderer
        .create_material(
            &MaterialDescriptor::new("grid", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Masked)
                .with_alpha_cutoff(0.35),
        )
        .unwrap();
    assert!(
        binding_lines(&journal)
            .pop()
            .unwrap()
            .contains(" cutoff=0.35")
    );
    let bindings_before = binding_lines(&journal).len();
    let pipelines_before = create_pipeline_lines(&journal).len();
    renderer.set_material_alpha_cutoff(grid, 0.7).unwrap();
    // L'écriture est IN-PLACE : un update au journal, aucun binding
    // ni pipeline créé, l'inspection reflète la valeur.
    assert!(
        journal
            .entries()
            .iter()
            .rfind(|entry| entry.starts_with("update_material_binding"))
            .unwrap()
            .contains(" cutoff=0.7")
    );
    assert_eq!(binding_lines(&journal).len(), bindings_before);
    assert_eq!(create_pipeline_lines(&journal).len(), pipelines_before);
    assert_eq!(renderer.material_info(grid).unwrap().alpha_cutoff, 0.7);
    // Refus nommés : hors Masked, hors bornes.
    let solid = renderer
        .create_material(&MaterialDescriptor::new("solid", MaterialModel::Lit))
        .unwrap();
    let refused = renderer.set_material_alpha_cutoff(solid, 0.3).unwrap_err();
    assert!(refused.to_string().contains("is not Masked"));
    let wild = renderer.set_material_alpha_cutoff(grid, 2.0).unwrap_err();
    assert!(wild.to_string().contains("0..=1"));
}

#[test]
fn transparent_draws_are_sorted_back_to_front() {
    let (mut renderer, journal) = mock_renderer();
    let glass = renderer
        .create_material(
            &MaterialDescriptor::new("glass", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
    let mesh = lit_quad_mesh(&mut renderer, "pane");
    for z in [-1.0, -5.0, -3.0] {
        renderer.queue_draw(DrawCommand {
            mesh,
            material: glass,
            transform: Transform::from_translation(Vec3::new(0.0, 0.0, z)),
        });
    }
    // Caméra à l'origine (défaut) : le plus LOINTAIN d'abord.
    renderer.render_frame().unwrap();
    let line = render_lines(&journal).pop().unwrap();
    let far = line.find("m=[0, 0, -5]").unwrap();
    let mid = line.find("m=[0, 0, -3]").unwrap();
    let near = line.find("m=[0, 0, -1]").unwrap();
    assert!(far < mid && mid < near);
}

#[test]
fn the_transparent_sort_follows_the_camera() {
    let (mut renderer, journal) = mock_renderer();
    let glass = renderer
        .create_material(
            &MaterialDescriptor::new("glass", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
    let mesh = lit_quad_mesh(&mut renderer, "pane");
    for z in [-1.0, -5.0] {
        renderer.queue_draw(DrawCommand {
            mesh,
            material: glass,
            transform: Transform::from_translation(Vec3::new(0.0, 0.0, z)),
        });
    }
    renderer.set_camera_position(Vec3::new(0.0, 0.0, 2.0));
    renderer.render_frame().unwrap();
    let first = render_lines(&journal).pop().unwrap();
    assert!(first.find("m=[0, 0, -5]").unwrap() < first.find("m=[0, 0, -1]").unwrap());
    // La caméra passe DERRIÈRE les panneaux : l'ordre s'inverse à la
    // frame suivante — le tri suit la caméra de la passe.
    renderer.set_camera_position(Vec3::new(0.0, 0.0, -8.0));
    renderer.render_frame().unwrap();
    let second = render_lines(&journal).pop().unwrap();
    assert!(second.find("m=[0, 0, -1]").unwrap() < second.find("m=[0, 0, -5]").unwrap());
}

#[test]
fn equal_depths_keep_the_submission_order() {
    let (mut renderer, journal) = mock_renderer();
    let veil = renderer
        .create_material(
            &MaterialDescriptor::new("veil", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
    let tint = renderer
        .create_material(
            &MaterialDescriptor::new("tint", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
    let mesh = lit_quad_mesh(&mut renderer, "pane");
    // Deux transparents à la MÊME distance : le tri stable conserve
    // l'ordre d'arrivée (regroupé par material par la file).
    for material in [tint, veil] {
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::from_translation(Vec3::new(0.0, 0.0, -2.0)),
        });
    }
    renderer.render_frame().unwrap();
    let line = render_lines(&journal).pop().unwrap();
    assert!(line.find("b=Some(0)").unwrap() < line.find("b=Some(1)").unwrap());
}

#[test]
fn the_pass_order_is_opaque_masked_sky_transparent() {
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
    let solid = renderer
        .create_material(&MaterialDescriptor::new("solid", MaterialModel::Lit))
        .unwrap();
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
    let mesh = lit_quad_mesh(&mut renderer, "pane");
    // Soumis à l'ENVERS de l'ordre de rendu : la partition remet
    // opaque → masked → ciel → transparent.
    for material in [glass, grid, solid] {
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
    }
    renderer.render_frame().unwrap();
    // Bindings dans l'ordre de création : solid=0, grid=1, glass=2 ;
    // le ciel est le tuple sans buffers (pipeline lazy, 3 sommets).
    let line = render_lines(&journal).pop().unwrap();
    let solid_at = line.find("b=Some(0)").unwrap();
    let grid_at = line.find("b=Some(1)").unwrap();
    let sky_at = line.find(", None, None, 3, b=None").unwrap();
    let glass_at = line.find("b=Some(2)").unwrap();
    assert!(solid_at < grid_at);
    assert!(grid_at < sky_at);
    assert!(sky_at < glass_at);
}

#[test]
fn the_breakdown_reports_the_categories() {
    let (mut renderer, _journal) = mock_renderer();
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
    let solid = renderer
        .create_material(&MaterialDescriptor::new("solid", MaterialModel::Lit))
        .unwrap();
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
    let mesh = lit_quad_mesh(&mut renderer, "pane");
    for material in [solid, solid, grid, glass] {
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
    }
    renderer.render_frame().unwrap();
    let report = renderer.frame_report();
    assert_eq!(
        report.passes[0].breakdown,
        DrawBreakdown {
            opaque: 2,
            masked: 1,
            transparent: 1,
            injected: 1,
        }
    );
    assert_eq!(report.passes[0].draws, 5);
    // Une passe désactivée rapporte une ventilation vide.
    let main = renderer.main_pass();
    renderer.set_pass_enabled(main, false).unwrap();
    renderer.render_frame().unwrap();
    assert_eq!(
        renderer.frame_report().passes[0].breakdown,
        DrawBreakdown::default()
    );
}

#[test]
fn masked_materials_cast_shadow_silhouettes() {
    let (mut renderer, _journal) = mock_renderer();
    renderer.set_directional_shadow(&demo_shadow()).unwrap();
    renderer.submit_light(Light::directional(Vec3::NEG_Y, Color::WHITE, 1.0));
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
    let mesh = lit_quad_mesh(&mut renderer, "pane");
    for material in [grid, glass] {
        renderer.queue_draw(DrawCommand {
            mesh,
            material,
            transform: Transform::IDENTITY,
        });
    }
    renderer.render_frame().unwrap();
    // Le masked projette (sa silhouette pleine V1), le transparent
    // jamais — le contrat de la catégorie.
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
fn checkpoint_transparency_v1_full_scene_over_two_frames() {
    // LE checkpoint Transparency & Ordering V1 : les trois
    // catégories sous ombres et ciel, l'ordre à quatre temps, le
    // tri qui SUIT la caméra entre deux frames, le cutoff retouché
    // à chaud, la ventilation exacte.
    let (mut renderer, journal) = mock_renderer();
    let cubemap = renderer
        .create_texture(&TextureDescriptor::cube(
            "chk.sky",
            1,
            TextureFormat::Rgba8Unorm,
            vec![0; 4 * 6],
        ))
        .unwrap();
    renderer
        .set_environment(&EnvironmentDescriptor::new(cubemap))
        .unwrap();
    renderer.set_directional_shadow(&demo_shadow()).unwrap();
    let solid = renderer
        .create_material(&MaterialDescriptor::new("chk.solid", MaterialModel::Lit))
        .unwrap();
    let grid = renderer
        .create_material(
            &MaterialDescriptor::new("chk.grid", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Masked)
                .with_alpha_cutoff(0.4),
        )
        .unwrap();
    let glass = renderer
        .create_material(
            &MaterialDescriptor::new("chk.glass", MaterialModel::Lit)
                .with_opacity(MaterialOpacity::Transparent),
        )
        .unwrap();
    let mesh = lit_quad_mesh(&mut renderer, "chk.pane");
    let sun = Light::directional(Vec3::NEG_Y, Color::WHITE, 0.9);
    let queue_scene = |renderer: &mut Renderer| {
        for (material, z) in [(solid, -4.0), (grid, -2.0), (glass, -1.0), (glass, -6.0)] {
            renderer.queue_draw(DrawCommand {
                mesh,
                material,
                transform: Transform::from_translation(Vec3::new(0.0, 0.0, z)),
            });
        }
    };

    // FRAME 1 — caméra devant : le verre LOINTAIN (-6) d'abord.
    renderer.set_camera_position(Vec3::new(0.0, 0.0, 2.0));
    renderer.submit_light(sun.clone());
    queue_scene(&mut renderer);
    renderer.render_frame().unwrap();
    let report = renderer.frame_report();
    assert_eq!(
        report.passes[0].breakdown,
        DrawBreakdown {
            opaque: 1,
            masked: 1,
            transparent: 2,
            injected: 1,
        }
    );
    // Opaque et masked projettent (2 casters), le transparent non.
    assert_eq!(
        report.shadow,
        Some(ShadowReport {
            draws: 2,
            draw_calls: 2,
            culled: 0,
            resolution: 2048
        })
    );
    let first = render_lines(&journal).pop().unwrap();
    assert!(first.find("m=[0, 0, -6]").unwrap() > first.find("m=[0, 0, -2]").unwrap());
    assert!(first.find("m=[0, 0, -6]").unwrap() < first.find("m=[0, 0, -1]").unwrap());

    // FRAME 2 — caméra DERRIÈRE la scène : l'ordre des verres
    // s'inverse ; le cutoff est retouché À CHAUD (in-place, aucune
    // recréation).
    let pipelines_before = create_pipeline_lines(&journal).len();
    renderer.set_material_alpha_cutoff(grid, 0.8).unwrap();
    renderer.clear_draws();
    renderer.set_camera_position(Vec3::new(0.0, 0.0, -12.0));
    renderer.submit_light(sun);
    queue_scene(&mut renderer);
    renderer.render_frame().unwrap();
    let second = render_lines(&journal).pop().unwrap();
    assert!(second.find("m=[0, 0, -1]").unwrap() < second.find("m=[0, 0, -6]").unwrap());
    assert_eq!(create_pipeline_lines(&journal).len(), pipelines_before);
    assert_eq!(renderer.material_info(grid).unwrap().alpha_cutoff, 0.8);
    assert_eq!(
        renderer.frame_report().passes[0].breakdown,
        DrawBreakdown {
            opaque: 1,
            masked: 1,
            transparent: 2,
            injected: 1,
        }
    );
}
