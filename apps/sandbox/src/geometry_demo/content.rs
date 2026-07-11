//! Le CONTENU PROCÉDURAL de la démo : les pixels générés par le code —
//! normal map, cubemap de ciel, grille ajourée. Aucun handle, aucun
//! état : des fonctions pures qui produisent des octets prêts pour les
//! descripteurs de textures.

use chaos_engine::{math::Vec3, rgba16f_bytes_of};

/// Une normal map procédurale : des bosses sinusoïdales encodées en
/// vecteurs tangent-space NORMALISÉS (+Y vert, convention glTF), format
/// linéaire, sans mips — le contenu de démonstration du normal mapping.
pub(super) fn bumpy_normal_map(size: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = x as f32 / size as f32 * std::f32::consts::TAU * 4.0;
            let v = y as f32 / size as f32 * std::f32::consts::TAU * 4.0;
            let normal = Vec3::new(0.55 * u.sin(), 0.55 * v.cos(), 1.0).normalize();
            let encode = |component: f32| ((component * 0.5 + 0.5) * 255.0).round() as u8;
            pixels.extend_from_slice(&[encode(normal.x), encode(normal.y), encode(normal.z), 255]);
        }
    }
    pixels
}

/// La direction monde d'un texel de cubemap — la convention wgpu des
/// faces (+X, -X, +Y, -Y, +Z, -Z), u/v dans [0, 1] depuis le coin haut
/// gauche de la face.
fn cube_direction(face: usize, u: f32, v: f32) -> Vec3 {
    let uc = 2.0 * u - 1.0;
    let vc = 2.0 * v - 1.0;
    let direction = match face {
        0 => Vec3::new(1.0, -vc, -uc),
        1 => Vec3::new(-1.0, -vc, uc),
        2 => Vec3::new(uc, 1.0, vc),
        3 => Vec3::new(uc, -1.0, -vc),
        4 => Vec3::new(uc, -vc, 1.0),
        _ => Vec3::new(-uc, -vc, -1.0),
    };
    direction.normalize()
}

/// La cubemap HDR procédurale du ciel : gradient zénith → horizon → sol
/// plus un disque solaire LARGEMENT au-delà de 1 (aligné sur la
/// directionnelle de la démo) — les valeurs HDR que l'IBL, le ciel et
/// l'exposition exploitent. Encodée pour `Rgba16Float`.
pub(super) fn sky_cubemap_pixels(size: u32) -> Vec<u8> {
    let sun_direction = Vec3::new(0.4, 1.0, 0.3).normalize();
    let zenith = Vec3::new(0.25, 0.45, 0.95) * 1.1;
    let horizon = Vec3::new(1.0, 0.85, 0.7);
    let ground = Vec3::new(0.12, 0.10, 0.09);
    let mut values = Vec::with_capacity((size * size * 4 * 6) as usize);
    for face in 0..6 {
        for y in 0..size {
            for x in 0..size {
                let u = (x as f32 + 0.5) / size as f32;
                let v = (y as f32 + 0.5) / size as f32;
                let direction = cube_direction(face, u, v);
                let up = direction.y.clamp(-1.0, 1.0);
                let base = if up >= 0.0 {
                    horizon.lerp(zenith, up.powf(0.6))
                } else {
                    horizon.lerp(ground, (-up).powf(0.4))
                };
                let disc = ((direction.dot(sun_direction) - 0.997) / 0.003).clamp(0.0, 1.0);
                let color = base + Vec3::new(12.0, 11.0, 9.0) * disc * disc;
                values.extend_from_slice(&[color.x, color.y, color.z, 1.0]);
            }
        }
    }
    rgba16f_bytes_of(&values)
}

/// La texture de la grille masked : un treillis opaque percé de
/// PASTILLES transparentes (alpha 0) — le contenu de démonstration de
/// l'alpha cutout. Octets sRGB directs, alpha straight.
pub(super) fn grille_pixels(size: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    let cells = 4.0;
    for y in 0..size {
        for x in 0..size {
            let u = ((x as f32 + 0.5) / size as f32 * cells).fract() - 0.5;
            let v = ((y as f32 + 0.5) / size as f32 * cells).fract() - 0.5;
            let alpha = if (u * u + v * v).sqrt() < 0.38 {
                0
            } else {
                255
            };
            pixels.extend_from_slice(&[214, 196, 160, alpha]);
        }
    }
    pixels
}
