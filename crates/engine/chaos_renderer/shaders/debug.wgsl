// Le shader du DEBUG RENDERING : des lignes monde pré-transformées —
// position + couleur RGBA par sommet, projetées par la seule
// vue-projection de la frame. Aucune matrice modèle (le groupe objet
// n'est pas lu), aucun material, aucune variante instanciée ou masked :
// le debug n'est pas un material. Le layout des entrées est l'autorité
// `DebugVertex::layout()` — verrouillé par test naga.

struct FrameUniforms {
    view_projection: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> frame: FrameUniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = frame.view_projection * vec4<f32>(input.position, 1.0);
    output.color = input.color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}
