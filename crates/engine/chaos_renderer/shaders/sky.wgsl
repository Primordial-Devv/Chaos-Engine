struct FrameUniforms {
    view_projection: mat4x4<f32>,
    camera_position: vec4<f32>,
    inverse_view_projection: mat4x4<f32>,
    environment_params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: FrameUniforms;
@group(0) @binding(2) var environment_map: texture_cube<f32>;
@group(0) @binding(3) var environment_sampler: sampler;

struct SkyOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

// Triangle plein écran généré depuis l'index, à la profondeur MAXIMALE
// (z = w = 1.0, convention DirectX 0..1) : sous LessEqual, le ciel ne
// couvre que les pixels laissés au clear.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> SkyOutput {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var output: SkyOutput;
    output.position = vec4<f32>(corners[index], 1.0, 1.0);
    output.ndc = corners[index];
    return output;
}

@fragment
fn fs_main(input: SkyOutput) -> @location(0) vec4<f32> {
    // La direction de vue par déprojection near → far : indépendante de
    // camera_position, correcte pour toute matrice vue-projection.
    let near = frame.inverse_view_projection * vec4<f32>(input.ndc, 0.0, 1.0);
    let far = frame.inverse_view_projection * vec4<f32>(input.ndc, 1.0, 1.0);
    let direction = normalize(far.xyz / far.w - near.xyz / near.w);
    let sample = textureSample(environment_map, environment_sampler, direction).rgb;
    let color = sample * frame.environment_params.x * frame.environment_params.y;
    // Le même Reinhard provisoire que chaos.pbr — ciel et scène cohérents.
    let mapped_color = color / (vec3<f32>(1.0) + color);
    return vec4<f32>(mapped_color, 1.0);
}
