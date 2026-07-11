struct FrameUniforms {
    view_projection: mat4x4<f32>,
};

struct ObjectUniforms {
    model: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> frame: FrameUniforms;
@group(1) @binding(0) var<uniform> object: ObjectUniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = frame.view_projection * object.model * vec4<f32>(input.position, 1.0);
    output.color = input.color;
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
    output.color = input.color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color, 1.0);
}
