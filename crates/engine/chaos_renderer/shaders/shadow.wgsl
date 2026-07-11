// Le shader de la passe d'OMBRE : profondeur seule — le vertex projette
// la position dans le clip de la LUMIÈRE (la caméra de la passe), aucun
// étage fragment. Le groupe(0) est le layout RÉDUIT (buffer frame seul) :
// la shadow map est l'ATTACHEMENT de cette passe, jamais une entrée.
struct FrameUniforms {
    view_projection: mat4x4<f32>,
};

struct ObjectUniforms {
    model: mat4x4<f32>,
    normal: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> frame: FrameUniforms;
@group(1) @binding(0) var<uniform> object: ObjectUniforms;

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return frame.view_projection * object.model * vec4<f32>(position, 1.0);
}

// L'entrée INSTANCIÉE des casters regroupés : la matrice modèle vient
// des attributs d'instance (locations 4..7) — la normale du layout
// d'instance n'est pas lue (la profondeur s'en passe).
struct InstanceModel {
    @location(4) model_0: vec4<f32>,
    @location(5) model_1: vec4<f32>,
    @location(6) model_2: vec4<f32>,
    @location(7) model_3: vec4<f32>,
};

@vertex
fn vs_instanced(
    @location(0) position: vec3<f32>,
    instance: InstanceModel,
) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(
        instance.model_0,
        instance.model_1,
        instance.model_2,
        instance.model_3,
    );
    return frame.view_projection * model * vec4<f32>(position, 1.0);
}
