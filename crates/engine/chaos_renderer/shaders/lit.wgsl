struct FrameUniforms {
    view_projection: mat4x4<f32>,
};

struct GpuLight {
    position_range: vec4<f32>,
    direction_kind: vec4<f32>,
    color_intensity: vec4<f32>,
    cone: vec4<f32>,
};

struct LightsUniforms {
    ambient: vec4<f32>,
    count: vec4<u32>,
    lights: array<GpuLight, 16>,
    shadow_view_projection: mat4x4<f32>,
    shadow_params: vec4<f32>,
};

struct ObjectUniforms {
    model: mat4x4<f32>,
    normal: mat4x4<f32>,
};

struct MaterialUniforms {
    base_color: vec4<f32>,
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: FrameUniforms;
@group(0) @binding(1) var<uniform> lights: LightsUniforms;
@group(0) @binding(4) var shadow_map: texture_depth_2d;
@group(0) @binding(5) var shadow_sampler: sampler_comparison;
@group(1) @binding(0) var<uniform> object: ObjectUniforms;
@group(2) @binding(0) var material_texture: texture_2d<f32>;
@group(2) @binding(1) var material_sampler: sampler;
@group(2) @binding(2) var<uniform> material: MaterialUniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    let world_position = object.model * vec4<f32>(input.position, 1.0);
    output.position = frame.view_projection * world_position;
    output.world_position = world_position.xyz;
    output.world_normal = (object.normal * vec4<f32>(input.normal, 0.0)).xyz;
    output.uv = input.uv;
    return output;
}

// L'entrée INSTANCIÉE : modèle et normale viennent des attributs
// d'instance (locations 4..7 et 8..11 — la convention
// `INSTANCE_LOCATION_BASE`), les ObjectUniforms ne sont pas lus.
// Mêmes sorties que vs_main.
struct InstanceTransforms {
    @location(4) model_0: vec4<f32>,
    @location(5) model_1: vec4<f32>,
    @location(6) model_2: vec4<f32>,
    @location(7) model_3: vec4<f32>,
    @location(8) normal_0: vec4<f32>,
    @location(9) normal_1: vec4<f32>,
    @location(10) normal_2: vec4<f32>,
    @location(11) normal_3: vec4<f32>,
};

@vertex
fn vs_instanced(input: VertexInput, instance: InstanceTransforms) -> VertexOutput {
    let model = mat4x4<f32>(
        instance.model_0,
        instance.model_1,
        instance.model_2,
        instance.model_3,
    );
    let normal = mat4x4<f32>(
        instance.normal_0,
        instance.normal_1,
        instance.normal_2,
        instance.normal_3,
    );
    var output: VertexOutput;
    let world_position = model * vec4<f32>(input.position, 1.0);
    output.position = frame.view_projection * world_position;
    output.world_position = world_position.xyz;
    output.world_normal = (normal * vec4<f32>(input.normal, 0.0)).xyz;
    output.uv = input.uv;
    return output;
}

// Le facteur d'ombre de la lumière projetante : 1 = éclairé, 0 = ombré.
// PCF 3x3 au sampler de comparaison — textureSampleCompareLevel n'a
// aucune contrainte d'uniformité, les gardes restent de simples
// branches. Le biais de normale (unités monde) écarte le point AVANT la
// projection ; le biais de profondeur est soustrait à la référence.
// Hors du volume de lumière (UV ou profondeur hors bornes) → 1.
fn shadow_factor(world_position: vec3<f32>, world_normal: vec3<f32>) -> f32 {
    if (lights.shadow_params.x < 0.5) {
        return 1.0;
    }
    let biased = world_position + world_normal * lights.shadow_params.y;
    let light_clip = lights.shadow_view_projection * vec4<f32>(biased, 1.0);
    let projected = light_clip.xyz / light_clip.w;
    let uv = projected.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    let reference = projected.z - lights.shadow_params.z;
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || reference >= 1.0) {
        return 1.0;
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(shadow_map));
    var lit = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel;
            lit += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + offset, reference);
        }
    }
    return lit / 9.0;
}

fn shade(input: VertexOutput) -> vec4<f32> {
    let albedo = textureSample(material_texture, material_sampler, input.uv)
        * material.base_color;
    let surface_normal = normalize(input.world_normal);
    // L'ombre n'atténue QUE la contribution directe de la lumière
    // projetante — l'ambiante reste (params.z du material = receive).
    var shadow = 1.0;
    if (material.params.z > 0.5) {
        shadow = shadow_factor(input.world_position, surface_normal);
    }
    var total = lights.ambient.rgb * lights.ambient.w;
    for (var index = 0u; index < lights.count.x; index++) {
        let light = lights.lights[index];
        let kind = light.direction_kind.w;
        var to_surface_light = -light.direction_kind.xyz;
        var attenuation = 1.0;
        if (kind > 0.5) {
            let to_light = light.position_range.xyz - input.world_position;
            let distance = length(to_light);
            to_surface_light = to_light / max(distance, 1e-4);
            let range = light.position_range.w;
            let falloff = clamp(1.0 - (distance / range) * (distance / range), 0.0, 1.0);
            attenuation = falloff * falloff;
            if (kind > 1.5) {
                let cone = smoothstep(
                    light.cone.y,
                    light.cone.x,
                    dot(-to_surface_light, light.direction_kind.xyz),
                );
                attenuation = attenuation * cone;
            }
        }
        let diffuse = max(dot(surface_normal, to_surface_light), 0.0);
        var visibility = 1.0;
        if (index == u32(lights.shadow_params.w)) {
            visibility = shadow;
        }
        total += light.color_intensity.rgb * light.color_intensity.w
            * diffuse * attenuation * visibility;
    }
    return vec4<f32>(albedo.rgb * total, albedo.a);
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
