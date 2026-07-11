use chaos_renderer::{ShaderLibrary, ShaderSource, shaders::inputs};

#[test]
fn builtin_shaders_follow_the_input_conventions() {
    let allowed = [
        (inputs::FRAME_GROUP, inputs::FRAME_UNIFORMS_BINDING),
        (inputs::FRAME_GROUP, inputs::FRAME_LIGHTS_BINDING),
        (inputs::FRAME_GROUP, inputs::FRAME_ENVIRONMENT_BINDING),
        (
            inputs::FRAME_GROUP,
            inputs::FRAME_ENVIRONMENT_SAMPLER_BINDING,
        ),
        (inputs::FRAME_GROUP, inputs::FRAME_SHADOW_BINDING),
        (inputs::FRAME_GROUP, inputs::FRAME_SHADOW_SAMPLER_BINDING),
        (inputs::OBJECT_GROUP, inputs::OBJECT_UNIFORMS_BINDING),
        (inputs::MATERIAL_GROUP, inputs::MATERIAL_TEXTURE_BINDING),
        (inputs::MATERIAL_GROUP, inputs::MATERIAL_SAMPLER_BINDING),
        (inputs::MATERIAL_GROUP, inputs::MATERIAL_UNIFORMS_BINDING),
        (
            inputs::MATERIAL_GROUP,
            inputs::MATERIAL_METALLIC_ROUGHNESS_BINDING,
        ),
        (inputs::MATERIAL_GROUP, inputs::MATERIAL_NORMAL_BINDING),
        (inputs::MATERIAL_GROUP, inputs::MATERIAL_OCCLUSION_BINDING),
        (inputs::MATERIAL_GROUP, inputs::MATERIAL_EMISSIVE_BINDING),
    ];
    let library = ShaderLibrary::with_builtins();
    let mut checked = 0;

    for (name, source) in library.iter() {
        let ShaderSource::Wgsl(code) = source;
        let module = naga::front::wgsl::parse_str(code)
            .unwrap_or_else(|error| panic!("shader '{name}' invalide : {error:?}"));
        for (_, variable) in module.global_variables.iter() {
            if let Some(resource) = &variable.binding {
                assert!(
                    allowed.contains(&(resource.group, resource.binding)),
                    "le shader intégré '{name}' déclare un binding hors convention : \
                     group({}) binding({}) — l'autorité est chaos_renderer::shaders::inputs",
                    resource.group,
                    resource.binding
                );
                checked += 1;
            }
        }
    }

    assert!(checked > 0, "aucun binding n'a été vérifié");
}

#[test]
fn the_lit_shader_capacity_matches_the_engine() {
    // MAX_LIGHTS vit en Rust ET dans les WGSL embarqués : ce verrou
    // casse si l'un bouge sans les autres.
    let library = ShaderLibrary::with_builtins();
    let expected = format!("array<GpuLight, {}>", chaos_renderer::MAX_LIGHTS);
    for name in [
        chaos_renderer::shaders::builtin::LIT,
        chaos_renderer::shaders::builtin::PBR,
    ] {
        let Some(ShaderSource::Wgsl(code)) = library.get(name) else {
            panic!("'{name}' is missing from the builtins");
        };
        assert!(
            code.contains(&expected),
            "'{name}' must declare {expected} to match chaos_renderer::MAX_LIGHTS"
        );
    }
}

#[test]
fn the_frame_uniforms_mirror_is_locked() {
    // FrameUniforms (160 octets) vit côté backend ET dans les WGSL qui
    // consomment l'environnement : aucun test GPU ne valide ce miroir,
    // ce verrou textuel casse si l'un bouge sans les autres.
    let library = ShaderLibrary::with_builtins();
    for name in [
        chaos_renderer::shaders::builtin::PBR,
        chaos_renderer::shaders::builtin::SKY,
    ] {
        let Some(ShaderSource::Wgsl(code)) = library.get(name) else {
            panic!("'{name}' is missing from the builtins");
        };
        for expected in [
            "inverse_view_projection: mat4x4<f32>",
            "environment_params: vec4<f32>",
        ] {
            assert!(
                code.contains(expected),
                "'{name}' must declare `{expected}` to mirror the backend FrameUniforms"
            );
        }
    }
    let Some(ShaderSource::Wgsl(sky)) = library.get(chaos_renderer::shaders::builtin::SKY) else {
        panic!("the sky shader is missing from the builtins");
    };
    assert!(
        sky.contains("texture_cube<f32>"),
        "the sky shader must sample the environment cubemap"
    );
}

#[test]
fn the_lights_uniforms_shadow_mirror_is_locked() {
    // La queue OMBRE de LightsUniforms (1 136 octets) vit côté backend ET
    // dans les WGSL éclairés : ce verrou textuel casse si l'un bouge sans
    // les autres — aucun test GPU n'existe.
    let library = ShaderLibrary::with_builtins();
    for name in [
        chaos_renderer::shaders::builtin::LIT,
        chaos_renderer::shaders::builtin::PBR,
    ] {
        let Some(ShaderSource::Wgsl(code)) = library.get(name) else {
            panic!("'{name}' is missing from the builtins");
        };
        for expected in [
            "shadow_view_projection: mat4x4<f32>",
            "shadow_params: vec4<f32>",
            "texture_depth_2d",
            "sampler_comparison",
        ] {
            assert!(
                code.contains(expected),
                "'{name}' must declare `{expected}` to mirror the backend LightsUniforms shadow tail"
            );
        }
    }
}

#[test]
fn the_shadow_shader_is_vertex_only_and_minimal() {
    // La passe d'ombre binde le groupe(0) RÉDUIT (buffer frame seul) :
    // son shader ne doit déclarer NI étage fragment NI binding au-delà de
    // (0,0) et (1,0) — un binding de plus casserait le layout réduit.
    let library = ShaderLibrary::with_builtins();
    let Some(ShaderSource::Wgsl(code)) = library.get(chaos_renderer::shaders::builtin::SHADOW)
    else {
        panic!("the shadow shader is missing from the builtins");
    };
    let module = naga::front::wgsl::parse_str(code)
        .unwrap_or_else(|error| panic!("shader 'chaos.shadow' invalide : {error:?}"));
    let entry_points: Vec<&str> = module
        .entry_points
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(
        entry_points,
        vec!["vs_main", "vs_instanced"],
        "the shadow shader must expose exactly its two vertex entry points"
    );
    for (_, variable) in module.global_variables.iter() {
        if let Some(resource) = &variable.binding {
            assert!(
                matches!(
                    (resource.group, resource.binding),
                    (inputs::FRAME_GROUP, inputs::FRAME_UNIFORMS_BINDING)
                        | (inputs::OBJECT_GROUP, inputs::OBJECT_UNIFORMS_BINDING)
                ),
                "the shadow shader must only bind the reduced frame group and the object group, \
                 found group({}) binding({})",
                resource.group,
                resource.binding
            );
        }
    }
}

#[test]
fn the_debug_shader_is_minimal_and_mirrors_its_layout() {
    // Le shader debug ne lit QUE la vue-projection ((0,0)) — jamais le
    // groupe objet (les sommets sont pré-transformés monde) — et ses
    // entrées vertex doivent refléter `DebugVertex::layout()` : deux
    // points d'entrée exactement, ni instancié, ni masked (le debug
    // n'est pas un material).
    let library = ShaderLibrary::with_builtins();
    let Some(ShaderSource::Wgsl(code)) = library.get(chaos_renderer::shaders::builtin::DEBUG)
    else {
        panic!("the debug shader is missing from the builtins");
    };
    let module = naga::front::wgsl::parse_str(code)
        .unwrap_or_else(|error| panic!("shader 'chaos.debug' invalide : {error:?}"));
    let entry_points: Vec<&str> = module
        .entry_points
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(
        entry_points,
        vec!["vs_main", "fs_main"],
        "the debug shader must expose exactly vs_main and fs_main"
    );
    for (_, variable) in module.global_variables.iter() {
        if let Some(resource) = &variable.binding {
            assert_eq!(
                (resource.group, resource.binding),
                (inputs::FRAME_GROUP, inputs::FRAME_UNIFORMS_BINDING),
                "the debug shader must only bind the frame uniforms"
            );
        }
    }
    // Le miroir du layout : @location(0) vec3 position, @location(1)
    // vec4 couleur — l'autorité est `DebugVertex::layout()`.
    let layout = chaos_renderer::DebugVertex::layout();
    assert_eq!(layout.stride, 28);
    assert_eq!(layout.attributes[0].location, 0);
    assert_eq!(layout.attributes[1].location, 1);
    assert!(code.contains("@location(0) position: vec3<f32>"));
    assert!(code.contains("@location(1) color: vec4<f32>"));
}

#[test]
fn the_masked_entry_is_locked() {
    // Les permutations Masked visent l'entrée `fs_masked` : les shaders
    // à entrées material doivent l'exposer À CÔTÉ de `fs_main` — un
    // material masked sur un modèle dont le shader ne l'a pas échouerait
    // à la création du pipeline, sur GPU seulement (aucun test GPU
    // n'existe : ce verrou est la garde).
    let library = ShaderLibrary::with_builtins();
    for name in [
        chaos_renderer::shaders::builtin::TEXTURED,
        chaos_renderer::shaders::builtin::LIT,
        chaos_renderer::shaders::builtin::PBR,
    ] {
        let Some(ShaderSource::Wgsl(code)) = library.get(name) else {
            panic!("'{name}' is missing from the builtins");
        };
        let module = naga::front::wgsl::parse_str(code)
            .unwrap_or_else(|error| panic!("shader '{name}' invalide : {error:?}"));
        let entry_points: Vec<&str> = module
            .entry_points
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        for expected in ["fs_main", "fs_masked"] {
            assert!(
                entry_points.contains(&expected),
                "'{name}' must expose the `{expected}` entry point"
            );
        }
    }
}

#[test]
fn the_instanced_entry_is_locked() {
    // Les permutations INSTANCIÉES visent l'entrée `vs_instanced` : les
    // cinq shaders à géométrie doivent l'exposer — un batch sur un
    // modèle dont le shader ne l'a pas échouerait à la création du
    // pipeline, sur GPU seulement (ce verrou est la garde).
    let library = ShaderLibrary::with_builtins();
    for name in [
        chaos_renderer::shaders::builtin::VERTEX_COLOR,
        chaos_renderer::shaders::builtin::TEXTURED,
        chaos_renderer::shaders::builtin::LIT,
        chaos_renderer::shaders::builtin::PBR,
        chaos_renderer::shaders::builtin::SHADOW,
    ] {
        let Some(ShaderSource::Wgsl(code)) = library.get(name) else {
            panic!("'{name}' is missing from the builtins");
        };
        let module = naga::front::wgsl::parse_str(code)
            .unwrap_or_else(|error| panic!("shader '{name}' invalide : {error:?}"));
        assert!(
            module
                .entry_points
                .iter()
                .any(|entry| entry.name == "vs_instanced"),
            "'{name}' must expose the `vs_instanced` entry point"
        );
    }
}

#[test]
fn builtin_shaders_are_valid_wgsl() {
    let library = ShaderLibrary::with_builtins();
    let mut checked = 0;

    for (name, source) in library.iter() {
        let ShaderSource::Wgsl(code) = source;

        let module = naga::front::wgsl::parse_str(code).unwrap_or_else(|error| {
            panic!(
                "le shader intégré '{name}' est un WGSL invalide :\n{}",
                error.emit_to_string(code)
            )
        });

        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|error| {
            panic!("le shader intégré '{name}' est rejeté par la validation naga : {error:?}")
        });

        checked += 1;
    }

    assert!(checked > 0, "aucun shader intégré n'a été vérifié");
}
