struct FrameUniforms {
    view_projection: mat4x4<f32>,
    camera_position: vec4<f32>,
    inverse_view_projection: mat4x4<f32>,
    environment_params: vec4<f32>,
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
    emissive: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: FrameUniforms;
@group(0) @binding(1) var<uniform> lights: LightsUniforms;
@group(0) @binding(2) var environment_map: texture_cube<f32>;
@group(0) @binding(3) var environment_sampler: sampler;
@group(0) @binding(4) var shadow_map: texture_depth_2d;
@group(0) @binding(5) var shadow_sampler: sampler_comparison;
@group(1) @binding(0) var<uniform> object: ObjectUniforms;
@group(2) @binding(0) var base_color_texture: texture_2d<f32>;
@group(2) @binding(1) var material_sampler: sampler;
@group(2) @binding(2) var<uniform> material: MaterialUniforms;
@group(2) @binding(3) var metallic_roughness_texture: texture_2d<f32>;
@group(2) @binding(4) var normal_texture: texture_2d<f32>;
@group(2) @binding(5) var occlusion_texture: texture_2d<f32>;
@group(2) @binding(6) var emissive_texture: texture_2d<f32>;

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

const PI: f32 = 3.14159265;

fn distribution_ggx(n_dot_h: f32, alpha: f32) -> f32 {
    let alpha2 = alpha * alpha;
    let denom = n_dot_h * n_dot_h * (alpha2 - 1.0) + 1.0;
    return alpha2 / max(PI * denom * denom, 1e-6);
}

fn geometry_smith(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    let r = roughness + 1.0;
    let k = (r * r) / 8.0;
    let view_term = n_dot_v / (n_dot_v * (1.0 - k) + k);
    let light_term = n_dot_l / (n_dot_l * (1.0 - k) + k);
    return view_term * light_term;
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
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

// L'approximation analytique de Karis (mobile) de la BRDF d'environnement
// — remplace la LUT en V1.
fn environment_brdf(f0: vec3<f32>, roughness: f32, n_dot_v: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

fn shade(input: VertexOutput, front_facing: bool) -> vec4<f32> {
    // TOUS les échantillonnages et les dérivées d'abord — l'analyse
    // d'uniformité interdit d'y toucher sous contrôle divergent.
    let albedo = textureSample(base_color_texture, material_sampler, input.uv)
        * material.base_color;
    let metallic_roughness =
        textureSample(metallic_roughness_texture, material_sampler, input.uv);
    let normal_sample = textureSample(normal_texture, material_sampler, input.uv).xyz;
    let occlusion = textureSample(occlusion_texture, material_sampler, input.uv).r;
    let emissive_sample = textureSample(emissive_texture, material_sampler, input.uv).rgb;
    let dp1 = dpdx(input.world_position);
    let dp2 = dpdy(input.world_position);
    let duv1 = dpdx(input.uv);
    let duv2 = dpdy(input.uv);

    var geometric_normal = normalize(input.world_normal);
    if (!front_facing) {
        geometric_normal = -geometric_normal;
    }

    // Le repère cotangent dérivé (Schüler) : les tangentes viennent des
    // dérivées écran — aucun attribut de vertex. Des UV dégénérés
    // retombent sur la normale géométrique, jamais des NaN.
    let dp2perp = cross(dp2, geometric_normal);
    let dp1perp = cross(geometric_normal, dp1);
    let tangent = dp2perp * duv1.x + dp1perp * duv2.x;
    let bitangent = dp2perp * duv1.y + dp1perp * duv2.y;
    let frame_max = max(dot(tangent, tangent), dot(bitangent, bitangent));
    var surface_normal = geometric_normal;
    if (frame_max > 1e-8) {
        let inv = inverseSqrt(frame_max);
        let tbn = mat3x3<f32>(tangent * inv, bitangent * inv, geometric_normal);
        let mapped = normal_sample * 2.0 - 1.0;
        surface_normal = normalize(tbn * mapped);
    }

    let metallic = clamp(material.params.x * metallic_roughness.b, 0.0, 1.0);
    let roughness = clamp(material.params.y * metallic_roughness.g, 0.045, 1.0);
    let f0 = mix(vec3<f32>(0.04), albedo.rgb, metallic);
    let view = normalize(frame.camera_position.xyz - input.world_position);
    let n_dot_v = max(dot(surface_normal, view), 1e-4);

    // L'ombre n'atténue QUE la contribution directe de la lumière
    // projetante — l'ambiante et l'IBL restent (params.z = receive).
    // Le biais suit la normale GÉOMÉTRIQUE : la normal map ne doit pas
    // pousser le point d'échantillonnage dans la surface.
    var shadow = 1.0;
    if (material.params.z > 0.5) {
        shadow = shadow_factor(input.world_position, geometric_normal);
    }

    var outgoing = vec3<f32>(0.0);
    for (var index = 0u; index < lights.count.x; index++) {
        let light = lights.lights[index];
        let kind = light.direction_kind.w;
        var to_light = -light.direction_kind.xyz;
        var attenuation = 1.0;
        if (kind > 0.5) {
            let offset = light.position_range.xyz - input.world_position;
            let distance = length(offset);
            to_light = offset / max(distance, 1e-4);
            let range = light.position_range.w;
            let falloff = clamp(1.0 - (distance / range) * (distance / range), 0.0, 1.0);
            attenuation = falloff * falloff;
            if (kind > 1.5) {
                let cone = smoothstep(
                    light.cone.y,
                    light.cone.x,
                    dot(-to_light, light.direction_kind.xyz),
                );
                attenuation = attenuation * cone;
            }
        }
        let n_dot_l = max(dot(surface_normal, to_light), 0.0);
        let radiance = light.color_intensity.rgb * light.color_intensity.w * attenuation;
        let halfway = normalize(view + to_light);
        let n_dot_h = max(dot(surface_normal, halfway), 0.0);
        let distribution = distribution_ggx(n_dot_h, roughness * roughness);
        let geometry = geometry_smith(n_dot_v, n_dot_l, roughness);
        let fresnel = fresnel_schlick(max(dot(halfway, view), 0.0), f0);
        let specular = (distribution * geometry * fresnel)
            / (4.0 * n_dot_v * n_dot_l + 1e-4);
        let diffuse_weight = (vec3<f32>(1.0) - fresnel) * (1.0 - metallic);
        var visibility = 1.0;
        if (index == u32(lights.shadow_params.w)) {
            visibility = shadow;
        }
        outgoing += (diffuse_weight * albedo.rgb / PI + specular)
            * radiance * n_dot_l * visibility;
    }

    // IBL V1 : mips box de la cubemap (échantillonnage à LOD explicite —
    // aucune contrainte d'uniformité), BRDF analytique. Sans environnement
    // le cube fallback est noir : contribution nulle, sans branche.
    let environment_intensity = frame.environment_params.x;
    let max_lod = f32(textureNumLevels(environment_map) - 1u);
    let reflection = reflect(-view, surface_normal);
    let specular_environment = textureSampleLevel(
        environment_map,
        environment_sampler,
        reflection,
        roughness * max_lod,
    ).rgb;
    let irradiance = textureSampleLevel(
        environment_map,
        environment_sampler,
        surface_normal,
        max_lod,
    ).rgb;
    let environment = (irradiance * albedo.rgb * (1.0 - metallic)
        + specular_environment * environment_brdf(f0, roughness, n_dot_v))
        * environment_intensity * occlusion;

    let ambient = lights.ambient.rgb * lights.ambient.w * albedo.rgb * occlusion;
    let emissive = emissive_sample * material.emissive.rgb;
    let exposure = frame.environment_params.y;
    let color = (ambient + environment + outgoing + emissive) * exposure;
    // Reinhard par material — PROVISOIRE : le tone mapping par frame
    // viendra avec le post-process.
    let mapped_color = color / (vec3<f32>(1.0) + color);
    return vec4<f32>(mapped_color, albedo.a);
}

@fragment
fn fs_main(input: VertexOutput, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    return shade(input, front_facing);
}

// L'entrée MASKED (alpha cutout) : le fragment sous le seuil
// (params.w) est ÉLIMINÉ. Le discard vient APRÈS shade() — les
// échantillonnages et les dérivées (le TBN écran) s'exécutent sous
// contrôle uniforme — et la sortie conservée est OPAQUE (alpha 1).
// fs_main reste sans discard : l'early-Z des pipelines opaques est
// préservé.
@fragment
fn fs_masked(input: VertexOutput, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let color = shade(input, front_facing);
    if (color.a < material.params.w) {
        discard;
    }
    return vec4<f32>(color.rgb, 1.0);
}
