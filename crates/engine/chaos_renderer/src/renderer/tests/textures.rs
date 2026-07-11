//! Buffers, textures et samplers : création, forwarding et validation,
//! mips et cubemaps, cache `get_or_create`, builtins et fallbacks protégés.

use super::*;

#[test]
fn provided_mips_reach_the_backend() {
    let (mut renderer, journal) = mock_renderer();
    let mut pixels = vec![0u8; 4 * 4 * 4];
    pixels.extend_from_slice(&[0u8; 2 * 2 * 4]);
    pixels.extend_from_slice(&[0u8; 4]);
    renderer
        .create_texture(
            &TextureDescriptor::sampled("mippee", 4, 4, TextureFormat::Rgba8Unorm, pixels)
                .with_mips(TextureMips::Provided(3)),
        )
        .unwrap();
    assert_eq!(
        journal.entries(),
        vec!["create_texture mippee 4x4 format=Rgba8Unorm usage=Sampled bytes=84 levels=3"]
    );
}

#[test]
fn generate_is_resolved_before_the_backend() {
    let (mut renderer, journal) = mock_renderer();
    renderer
        .create_texture(
            &TextureDescriptor::sampled("auto", 2, 2, TextureFormat::Rgba8Unorm, vec![0; 16])
                .with_mips(TextureMips::Generate),
        )
        .unwrap();
    assert_eq!(
        journal.entries(),
        vec!["create_texture auto 2x2 format=Rgba8Unorm usage=Sampled bytes=20 levels=2"]
    );
}

#[test]
fn a_cubemap_reaches_the_backend_with_its_layers() {
    let (mut renderer, journal) = mock_renderer();
    renderer
        .create_texture(&TextureDescriptor::cube(
            "env",
            2,
            TextureFormat::Rgba16Float,
            vec![0; 2 * 2 * 8 * 6],
        ))
        .unwrap();
    assert_eq!(
        journal.entries(),
        vec!["create_texture env 2x2 format=Rgba16Float usage=Sampled bytes=192 kind=Cube"]
    );
}

#[test]
fn update_texture_is_validated_then_forwarded() {
    let (mut renderer, journal) = mock_renderer();
    let texture = small_texture(&mut renderer, "dynamique");
    renderer.update_texture(texture, &[42]).unwrap();
    assert!(
        journal
            .entries()
            .contains(&String::from("update_texture index=0 bytes=1"))
    );

    let wrong_len = renderer.update_texture(texture, &[1, 2, 3]).unwrap_err();
    assert!(wrong_len.to_string().contains("expects 1 bytes"));

    renderer.destroy_texture(texture).unwrap();
    let stale = renderer.update_texture(texture, &[42]).unwrap_err();
    assert!(stale.to_string().contains("stale"));
}

#[test]
fn mipmapped_and_cube_textures_refuse_updates() {
    let (mut renderer, _journal) = mock_renderer();
    let mipped = renderer
        .create_texture(
            &TextureDescriptor::sampled("mippee", 2, 2, TextureFormat::Rgba8Unorm, vec![0; 16])
                .with_mips(TextureMips::Generate),
        )
        .unwrap();
    let error = renderer.update_texture(mipped, &[0; 16]).unwrap_err();
    assert!(error.to_string().contains("single-level 2D"));

    let cube = renderer
        .create_texture(&TextureDescriptor::cube(
            "env",
            1,
            TextureFormat::Rgba8Unorm,
            vec![0; 24],
        ))
        .unwrap();
    let error = renderer.update_texture(cube, &[0; 24]).unwrap_err();
    assert!(error.to_string().contains("single-level 2D"));
}

#[test]
fn a_material_refuses_a_cubemap_texture() {
    let (mut renderer, _journal) = mock_renderer();
    let cube = renderer
        .create_texture(&TextureDescriptor::cube(
            "env",
            1,
            TextureFormat::Rgba8Unorm,
            vec![0; 24],
        ))
        .unwrap();
    let error = renderer
        .create_material(&MaterialDescriptor::new("m", MaterialModel::Unlit).with_texture(cube))
        .unwrap_err();
    assert!(error.to_string().contains("cubemap"));
    assert!(error.to_string().contains("environment pass"));
}

#[test]
fn builtin_textures_are_lazy_shared_and_protected() {
    let (mut renderer, journal) = mock_renderer();
    let white = renderer.builtin_texture(BuiltinTexture::White).unwrap();
    let white_again = renderer.builtin_texture(BuiltinTexture::White).unwrap();
    let black = renderer.builtin_texture(BuiltinTexture::Black).unwrap();
    let normal = renderer
        .builtin_texture(BuiltinTexture::FlatNormal)
        .unwrap();
    assert_eq!(white, white_again);
    assert_ne!(white, black);
    assert_ne!(black, normal);
    assert_eq!(
        journal
            .entries()
            .iter()
            .filter(|entry| entry.starts_with("create_texture chaos."))
            .count(),
        3
    );
    for handle in [white, black, normal] {
        let error = renderer.destroy_texture(handle).unwrap_err();
        assert!(error.to_string().contains("builtin fallback"));
        let error = renderer.update_texture(handle, &[0; 4]).unwrap_err();
        assert!(error.to_string().contains("builtin fallback"));
    }
}

#[test]
fn stats_count_the_full_mip_chain_bytes() {
    let (mut renderer, _journal) = mock_renderer();
    renderer
        .create_texture(
            &TextureDescriptor::sampled("mippee", 2, 2, TextureFormat::Rgba8Unorm, vec![0; 16])
                .with_mips(TextureMips::Generate),
        )
        .unwrap();
    assert_eq!(renderer.resource_stats().textures.bytes, 20);
}

#[test]
fn an_anisotropic_sampler_is_validated_before_the_backend() {
    let (mut renderer, journal) = mock_renderer();
    let error = renderer
        .create_sampler(&SamplerDescriptor::new("bad").with_anisotropy(4))
        .unwrap_err();
    assert!(error.to_string().contains("Linear filtering everywhere"));
    assert!(journal.entries().is_empty());

    renderer
        .create_sampler(
            &SamplerDescriptor::new("aniso")
                .with_mip_filter(SamplerFilter::Linear)
                .with_anisotropy(8),
        )
        .unwrap();
    assert_eq!(
        journal.entries(),
        vec!["create_sampler aniso filter=Linear address=Repeat mips=Linear aniso=8"]
    );
}

#[test]
fn create_buffer_forwards_descriptor_and_returns_distinct_handles() {
    let (mut renderer, journal) = mock_renderer();
    let first = renderer
        .create_buffer(&BufferDescriptor::vertex("tri", vec![0, 1, 2, 3]))
        .unwrap();
    let second = renderer
        .create_buffer(&BufferDescriptor::index("idx", vec![0, 1]))
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(
        journal.entries(),
        vec![
            "create_buffer tri kind=Vertex bytes=4",
            "create_buffer idx kind=Index bytes=2"
        ]
    );
}

#[test]
fn destroy_buffer_forwards_the_handle() {
    let (mut renderer, journal) = mock_renderer();
    let handle = renderer
        .create_buffer(&BufferDescriptor::vertex("tri", Vec::new()))
        .unwrap();
    renderer.destroy_buffer(handle).unwrap();
    assert_eq!(
        journal.entries(),
        vec!["create_buffer tri kind=Vertex bytes=0"]
    );
    renderer.render_frame().unwrap();
    assert_eq!(
        journal.entries().last().map(String::as_str),
        Some("destroy_buffer index=0")
    );
}

#[test]
fn create_texture_forwards_descriptor_and_returns_distinct_handles() {
    let (mut renderer, journal) = mock_renderer();
    let first = renderer
        .create_texture(&TextureDescriptor::sampled(
            "albedo",
            2,
            2,
            TextureFormat::Rgba8UnormSrgb,
            vec![255; 16],
        ))
        .unwrap();
    let second = renderer
        .create_texture(&TextureDescriptor::render_target(
            "offscreen",
            4,
            4,
            TextureFormat::Rgba8Unorm,
        ))
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(
        journal.entries(),
        vec![
            "create_texture albedo 2x2 format=Rgba8UnormSrgb usage=Sampled bytes=16",
            "create_texture offscreen 4x4 format=Rgba8Unorm usage=RenderTarget bytes=0"
        ]
    );
}

#[test]
fn destroy_texture_forwards_the_handle() {
    let (mut renderer, journal) = mock_renderer();
    let handle = renderer
        .create_texture(&TextureDescriptor::sampled(
            "mask",
            1,
            1,
            TextureFormat::R8Unorm,
            vec![128],
        ))
        .unwrap();
    renderer.destroy_texture(handle).unwrap();
    assert_eq!(
        journal.entries(),
        vec!["create_texture mask 1x1 format=R8Unorm usage=Sampled bytes=1"]
    );
    renderer.render_frame().unwrap();
    assert_eq!(
        journal.entries().last().map(String::as_str),
        Some("destroy_texture index=0")
    );
}

#[test]
fn texture_with_wrong_pixel_size_is_rejected_before_the_backend() {
    let (mut renderer, journal) = mock_renderer();
    let error = renderer
        .create_texture(&TextureDescriptor::sampled(
            "bad",
            2,
            2,
            TextureFormat::Rgba8Unorm,
            vec![0; 3],
        ))
        .unwrap_err();
    assert!(error.to_string().contains("16 bytes"));
    assert!(error.to_string().contains("got 3"));
    assert!(journal.entries().is_empty());
}

#[test]
fn render_target_with_initial_pixels_is_rejected() {
    let (mut renderer, journal) = mock_renderer();
    let mut descriptor = TextureDescriptor::render_target("rt", 2, 2, TextureFormat::Rgba8Unorm);
    descriptor.pixels = vec![0; 16];
    let error = renderer.create_texture(&descriptor).unwrap_err();
    assert!(error.to_string().contains("render target"));
    assert!(journal.entries().is_empty());
}

#[test]
fn zero_sized_texture_is_rejected() {
    let (mut renderer, journal) = mock_renderer();
    let error = renderer
        .create_texture(&TextureDescriptor::sampled(
            "empty",
            0,
            4,
            TextureFormat::R8Unorm,
            Vec::new(),
        ))
        .unwrap_err();
    assert!(error.to_string().contains("zero dimensions"));
    assert!(journal.entries().is_empty());
}

#[test]
fn create_sampler_forwards_descriptor_and_returns_distinct_handles() {
    let (mut renderer, journal) = mock_renderer();
    let first = renderer
        .create_sampler(&SamplerDescriptor::new("linear"))
        .unwrap();
    let second = renderer
        .create_sampler(
            &SamplerDescriptor::new("pixel")
                .with_filter(SamplerFilter::Nearest)
                .with_address_mode(SamplerAddressMode::ClampToEdge),
        )
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(
        journal.entries(),
        vec![
            "create_sampler linear filter=Linear address=Repeat",
            "create_sampler pixel filter=Nearest address=ClampToEdge"
        ]
    );
}

#[test]
fn destroy_sampler_forwards_the_handle() {
    let (mut renderer, journal) = mock_renderer();
    let handle = renderer
        .create_sampler(&SamplerDescriptor::new("s"))
        .unwrap();
    renderer.destroy_sampler(handle).unwrap();
    assert_eq!(
        journal.entries(),
        vec!["create_sampler s filter=Linear address=Repeat"]
    );
    renderer.render_frame().unwrap();
    assert_eq!(
        journal.entries().last().map(String::as_str),
        Some("destroy_sampler index=0")
    );
}

#[test]
fn get_or_create_texture_deduplicates_by_label() {
    let (mut renderer, journal) = mock_renderer();
    let descriptor = TextureDescriptor::sampled("shared", 1, 1, TextureFormat::R8Unorm, vec![255]);
    let first = renderer.get_or_create_texture(&descriptor).unwrap();
    let second = renderer.get_or_create_texture(&descriptor).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        journal.entries(),
        vec!["create_texture shared 1x1 format=R8Unorm usage=Sampled bytes=1"]
    );
}

#[test]
fn get_or_create_texture_recreates_after_destroy() {
    let (mut renderer, journal) = mock_renderer();
    let descriptor = TextureDescriptor::sampled("shared", 1, 1, TextureFormat::R8Unorm, vec![255]);
    let first = renderer.get_or_create_texture(&descriptor).unwrap();
    renderer.destroy_texture(first).unwrap();
    let second = renderer.get_or_create_texture(&descriptor).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        journal.entries(),
        vec![
            "create_texture shared 1x1 format=R8Unorm usage=Sampled bytes=1",
            "create_texture shared 1x1 format=R8Unorm usage=Sampled bytes=1"
        ]
    );
    renderer.render_frame().unwrap();
    assert!(
        journal
            .entries()
            .contains(&String::from("destroy_texture index=0"))
    );
}

#[test]
fn distinct_labels_create_distinct_textures() {
    let (mut renderer, _journal) = mock_renderer();
    let first = renderer
        .get_or_create_texture(&TextureDescriptor::sampled(
            "a",
            1,
            1,
            TextureFormat::R8Unorm,
            vec![255],
        ))
        .unwrap();
    let second = renderer
        .get_or_create_texture(&TextureDescriptor::sampled(
            "b",
            1,
            1,
            TextureFormat::R8Unorm,
            vec![255],
        ))
        .unwrap();
    assert_ne!(first, second);
}

#[test]
fn builtin_fallbacks_are_protected() {
    let (mut renderer, journal) = mock_renderer();
    renderer
        .create_material(&MaterialDescriptor::new("a", MaterialModel::Unlit))
        .unwrap();
    let fallback_texture = TextureHandle {
        index: 0,
        generation: 0,
    };
    let texture_error = renderer.destroy_texture(fallback_texture).unwrap_err();
    assert!(texture_error.to_string().contains("builtin fallback"));
    let fallback_sampler = SamplerHandle {
        index: 0,
        generation: 0,
    };
    let sampler_error = renderer.destroy_sampler(fallback_sampler).unwrap_err();
    assert!(sampler_error.to_string().contains("builtin fallback"));
    renderer
        .create_material(&MaterialDescriptor::new("b", MaterialModel::Unlit))
        .unwrap();
    let entries = journal.entries();
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.contains("create_texture chaos.white"))
            .count(),
        1
    );
}

#[test]
fn create_material_uses_builtin_fallbacks_once() {
    let (mut renderer, journal) = mock_renderer();
    renderer
        .create_material(&MaterialDescriptor::new("a", MaterialModel::Unlit))
        .unwrap();
    renderer
        .create_material(&MaterialDescriptor::new("b", MaterialModel::Unlit))
        .unwrap();
    let entries = journal.entries();
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.contains("chaos.white"))
            .count(),
        1
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.contains("chaos.default_sampler"))
            .count(),
        1
    );
    // Les slots PBR sont TOUJOURS remplis : la blanche (idx 0)
    // partagée par base/mr/ao/émissif, la normale plate (idx 1).
    assert_eq!(
        entries[entries.len() - 1],
        "create_material_binding b texture=0 sampler=0 color=(1, 1, 1, 1) mr=0 normal=1 ao=0 em=0"
    );
}
