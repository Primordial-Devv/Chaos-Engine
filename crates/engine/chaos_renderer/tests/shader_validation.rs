use chaos_renderer::{ShaderLibrary, ShaderSource};

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
