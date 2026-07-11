//! Le cycle de vie des ressources : destructions refusées tant que des
//! parts existent, retraite différée au point sûr, stats et churn intensif.

use super::*;

#[test]
fn destroying_a_used_texture_is_refused_naming_the_dependents() {
    let (mut renderer, _journal) = mock_renderer();
    let texture = small_texture(&mut renderer, "damier");
    let sampler = renderer
        .create_sampler(&SamplerDescriptor::new("s"))
        .unwrap();
    textured_material(&mut renderer, "a", texture, sampler);
    textured_material(&mut renderer, "b", texture, sampler);
    let error = renderer.destroy_texture(texture).unwrap_err();
    assert!(error.to_string().contains("'damier'"));
    assert!(error.to_string().contains("2 material(s)"));
}

#[test]
fn destroying_a_used_sampler_is_refused() {
    let (mut renderer, _journal) = mock_renderer();
    let texture = small_texture(&mut renderer, "t");
    let sampler = renderer
        .create_sampler(&SamplerDescriptor::new("lecture"))
        .unwrap();
    textured_material(&mut renderer, "a", texture, sampler);
    let error = renderer.destroy_sampler(sampler).unwrap_err();
    assert!(error.to_string().contains("'lecture'"));
    assert!(error.to_string().contains("still used"));
}

#[test]
fn destroying_a_mesh_owned_buffer_is_refused() {
    let (mut renderer, _journal) = mock_renderer();
    renderer.create_mesh("quad", &quad()).unwrap();
    let owned = BufferHandle {
        index: 0,
        generation: 0,
    };
    let error = renderer.destroy_buffer(owned).unwrap_err();
    assert!(error.to_string().contains("owned by mesh 'quad'"));
    assert!(error.to_string().contains("destroy the mesh instead"));
}

#[test]
fn destroying_the_material_releases_its_shares() {
    let (mut renderer, _journal) = mock_renderer();
    let texture = small_texture(&mut renderer, "t");
    let sampler = renderer
        .create_sampler(&SamplerDescriptor::new("s"))
        .unwrap();
    let material = textured_material(&mut renderer, "a", texture, sampler);
    assert!(renderer.destroy_texture(texture).is_err());
    renderer.destroy_material(material).unwrap();
    renderer.destroy_texture(texture).unwrap();
    renderer.destroy_sampler(sampler).unwrap();
}

#[test]
fn sharing_counts_every_consumer() {
    let (mut renderer, _journal) = mock_renderer();
    let texture = small_texture(&mut renderer, "partagee");
    let sampler = renderer
        .create_sampler(&SamplerDescriptor::new("s"))
        .unwrap();
    let first = textured_material(&mut renderer, "a", texture, sampler);
    let second = textured_material(&mut renderer, "b", texture, sampler);
    renderer.destroy_material(first).unwrap();
    let error = renderer.destroy_texture(texture).unwrap_err();
    assert!(error.to_string().contains("1 material(s)"));
    renderer.destroy_material(second).unwrap();
    renderer.destroy_texture(texture).unwrap();
}

#[test]
fn backend_release_is_deferred_to_the_end_of_the_next_frame() {
    let (mut renderer, journal) = mock_renderer();
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    renderer.destroy_mesh(mesh).unwrap();
    assert!(
        !journal
            .entries()
            .iter()
            .any(|entry| entry.starts_with("destroy_buffer"))
    );
    renderer.render_frame().unwrap();
    let entries = journal.entries();
    let render_position = entries
        .iter()
        .position(|entry| entry.starts_with("render"))
        .unwrap();
    let destroy_position = entries
        .iter()
        .position(|entry| entry.starts_with("destroy_buffer"))
        .unwrap();
    assert!(destroy_position > render_position);
}

#[test]
fn retired_resources_drain_after_a_frame() {
    let (mut renderer, _journal) = mock_renderer();
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    let texture = small_texture(&mut renderer, "t");
    renderer.destroy_mesh(mesh).unwrap();
    renderer.destroy_texture(texture).unwrap();
    assert_eq!(renderer.resource_stats().retired, 2);
    renderer.render_frame().unwrap();
    assert_eq!(renderer.resource_stats().retired, 0);
}

#[test]
fn resource_stats_count_bytes_exactly() {
    let (mut renderer, _journal) = mock_renderer();
    renderer
        .create_buffer(&BufferDescriptor::vertex("v", vec![0; 24]))
        .unwrap();
    renderer
        .create_texture(&TextureDescriptor::sampled(
            "t",
            2,
            2,
            TextureFormat::Rgba8UnormSrgb,
            vec![0; 16],
        ))
        .unwrap();
    let stats = renderer.resource_stats();
    assert_eq!(stats.buffers.alive, 1);
    assert_eq!(stats.buffers.bytes, 24);
    assert_eq!(stats.textures.alive, 1);
    assert_eq!(stats.textures.bytes, 16);
    assert_eq!(stats.estimated_bytes, 40);
}

#[test]
fn stats_return_to_baseline_after_destruction() {
    let (mut renderer, _journal) = mock_renderer();
    // Les fallbacks (blanche, normale plate) sont PROTÉGÉS et
    // persistent : la baseline se prend après leur résolution.
    renderer.builtin_texture(BuiltinTexture::White).unwrap();
    renderer
        .builtin_texture(BuiltinTexture::FlatNormal)
        .unwrap();
    let baseline = renderer.resource_stats();
    let texture = small_texture(&mut renderer, "t");
    let sampler = renderer
        .create_sampler(&SamplerDescriptor::new("s"))
        .unwrap();
    let material = textured_material(&mut renderer, "m", texture, sampler);
    let mesh = renderer.create_mesh("tri", &triangle()).unwrap();
    renderer.destroy_material(material).unwrap();
    renderer.destroy_mesh(mesh).unwrap();
    renderer.destroy_texture(texture).unwrap();
    renderer.destroy_sampler(sampler).unwrap();
    renderer.render_frame().unwrap();
    let stats = renderer.resource_stats();
    assert_eq!(stats.buffers, baseline.buffers);
    assert_eq!(stats.textures, baseline.textures);
    assert_eq!(stats.samplers, baseline.samplers);
    assert_eq!(stats.meshes, baseline.meshes);
    assert_eq!(stats.materials, baseline.materials);
    assert_eq!(stats.retired, 0);
    assert_eq!(stats.estimated_bytes, baseline.estimated_bytes);
}

#[test]
fn double_destroy_stays_an_explicit_stale_error() {
    let (mut renderer, _journal) = mock_renderer();
    let texture = small_texture(&mut renderer, "t");
    let sampler = renderer
        .create_sampler(&SamplerDescriptor::new("s"))
        .unwrap();
    let buffer = renderer
        .create_buffer(&BufferDescriptor::vertex("v", vec![1]))
        .unwrap();
    renderer.destroy_texture(texture).unwrap();
    renderer.destroy_sampler(sampler).unwrap();
    renderer.destroy_buffer(buffer).unwrap();
    assert!(
        renderer
            .destroy_texture(texture)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert!(
        renderer
            .destroy_sampler(sampler)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert!(
        renderer
            .destroy_buffer(buffer)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
}

#[test]
fn pipelines_are_counted_permanent() {
    let (mut renderer, _journal) = mock_renderer();
    renderer.create_pipeline(&inline_descriptor("a")).unwrap();
    renderer.create_pipeline(&inline_descriptor("b")).unwrap();
    assert_eq!(renderer.resource_stats().pipelines, 2);
}

#[test]
fn intensive_churn_never_leaks_nor_resolves_stale() {
    let (mut renderer, _journal) = mock_renderer();
    renderer.builtin_texture(BuiltinTexture::White).unwrap();
    renderer
        .builtin_texture(BuiltinTexture::FlatNormal)
        .unwrap();
    let baseline = renderer.resource_stats();
    let mut past_materials = Vec::new();
    let mut past_meshes = Vec::new();
    for cycle in 0..100 {
        let texture = small_texture(&mut renderer, &format!("t{cycle}"));
        let sampler = renderer
            .create_sampler(&SamplerDescriptor::new(format!("s{cycle}")))
            .unwrap();
        let first = textured_material(&mut renderer, &format!("a{cycle}"), texture, sampler);
        let second = textured_material(&mut renderer, &format!("b{cycle}"), texture, sampler);
        let mesh = renderer
            .create_mesh(&format!("m{cycle}"), &triangle())
            .unwrap();
        renderer.queue_draw(DrawCommand {
            mesh,
            material: first,
            transform: Transform::IDENTITY,
        });
        renderer.queue_draw(DrawCommand {
            mesh,
            material: second,
            transform: Transform::IDENTITY,
        });
        renderer.render_frame().unwrap();
        assert!(renderer.destroy_texture(texture).is_err());
        renderer.destroy_material(first).unwrap();
        renderer.destroy_material(second).unwrap();
        renderer.destroy_mesh(mesh).unwrap();
        renderer.destroy_texture(texture).unwrap();
        renderer.destroy_sampler(sampler).unwrap();
        renderer.clear_draws();
        renderer.render_frame().unwrap();
        past_materials.push(first);
        past_meshes.push(mesh);
    }
    let stats = renderer.resource_stats();
    assert_eq!(stats.buffers, baseline.buffers);
    assert_eq!(stats.textures, baseline.textures);
    assert_eq!(stats.samplers, baseline.samplers);
    assert_eq!(stats.meshes, baseline.meshes);
    assert_eq!(stats.materials, baseline.materials);
    assert_eq!(stats.retired, 0);
    for material in past_materials {
        assert!(renderer.destroy_material(material).is_err());
    }
    for mesh in past_meshes {
        assert!(renderer.destroy_mesh(mesh).is_err());
    }
}
