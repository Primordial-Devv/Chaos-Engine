struct FrameUniforms {
    view_projection: mat4x4<f32>,
};

struct ObjectUniforms {
    model: mat4x4<f32>,
};

struct MaterialUniforms {
    base_color: vec4<f32>,
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: FrameUniforms;
@group(1) @binding(0) var<uniform> object: ObjectUniforms;
@group(2) @binding(0) var material_texture: texture_2d<f32>;
@group(2) @binding(1) var material_sampler: sampler;
@group(2) @binding(2) var<uniform> material: MaterialUniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = frame.view_projection * object.model * vec4<f32>(input.position, 1.0);
    output.uv = input.uv;
    return output;
}

// L'entrée INSTANCIÉE : la matrice modèle vient des attributs d'instance
// (locations 4..7 — la convention `INSTANCE_LOCATION_BASE`), les
// ObjectUniforms ne sont pas lus. Mêmes sorties que vs_main.
struct InstanceModel {
    @location(4) model_0: vec4<f32>,
    @location(5) model_1: vec4<f32>,
    @location(6) model_2: vec4<f32>,
    @location(7) model_3: vec4<f32>,
};

@vertex
fn vs_instanced(input: VertexInput, instance: InstanceModel) -> VertexOutput {
    let model = mat4x4<f32>(
        instance.model_0,
        instance.model_1,
        instance.model_2,
        instance.model_3,
    );
    var output: VertexOutput;
    output.position = frame.view_projection * model * vec4<f32>(input.position, 1.0);
    output.uv = input.uv;
    return output;
}

fn shade(input: VertexOutput) -> vec4<f32> {
    return textureSample(material_texture, material_sampler, input.uv) * material.base_color;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return shade(input);
}

// L'entrée MASKED (alpha cutout) : le fragment sous le seuil
// (params.w) est ÉLIMINÉ. Le discard vient APRÈS shade() — les
// échantillonnages s'exécutent sous contrôle uniforme — et la sortie
// conservée est OPAQUE (alpha 1). fs_main reste sans discard :
// l'early-Z des pipelines opaques est préservé.
@fragment
fn fs_masked(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = shade(input);
    if (color.a < material.params.w) {
        discard;
    }
    return vec4<f32>(color.rgb, 1.0);
}
