use chaos_renderer::{ShaderLibrary, ShaderSource, shaders::inputs};

#[test]
fn builtin_shaders_follow_the_input_conventions() {
    let allowed = [
        (inputs::FRAME_GROUP, inputs::FRAME_UNIFORMS_BINDING),
        (inputs::OBJECT_GROUP, inputs::OBJECT_UNIFORMS_BINDING),
        (inputs::MATERIAL_GROUP, inputs::MATERIAL_TEXTURE_BINDING),
        (inputs::MATERIAL_GROUP, inputs::MATERIAL_SAMPLER_BINDING),
        (inputs::MATERIAL_GROUP, inputs::MATERIAL_UNIFORMS_BINDING),
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
