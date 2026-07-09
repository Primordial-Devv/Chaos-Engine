use chaos_core::{ChaosError, ChaosResult};
use gltf::Gltf;
use gltf::mesh::Mode;

use crate::import::{AssetImporter, ImportedAsset, MeshData};
use crate::registry::AssetKind;

/// Importeur glTF (`.glb` et `.gltf` auto-suffisant) — le format 3D de
/// référence du moteur, nativement aligné sur ses conventions (main droite,
/// +Y haut, -Z avant : zéro conversion de repère). Les buffers viennent du
/// chunk binaire GLB ou de data URIs base64 embarquées ; les fichiers
/// externes (`.bin`) exigeront la résolution de dépendances entre assets —
/// erreur explicite en attendant. Périmètre V1 : la première primitive
/// TRIANGLES du premier mesh, positions + UV + indices ; les normales
/// arriveront avec le vertex éclairé, les matériaux glTF avec leur
/// sous-phase.
pub struct GltfImporter;

impl AssetImporter for GltfImporter {
    fn kind(&self) -> AssetKind {
        AssetKind::Mesh
    }

    fn extensions(&self) -> &[&str] {
        &["glb", "gltf"]
    }

    fn import(&self, name: &str, bytes: &[u8]) -> ChaosResult<ImportedAsset> {
        parse_gltf(name, bytes).map(ImportedAsset::Mesh)
    }
}

fn parse_gltf(name: &str, bytes: &[u8]) -> ChaosResult<MeshData> {
    let document = Gltf::from_slice(bytes).map_err(|gltf_error| {
        ChaosError::Asset(format!("mesh '{name}' is not valid glTF: {gltf_error}"))
    })?;
    let blob = document.blob.as_deref();
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    for buffer in document.buffers() {
        let data = match buffer.source() {
            gltf::buffer::Source::Bin => match blob {
                Some(blob) => blob.to_vec(),
                None => {
                    return Err(ChaosError::Asset(format!(
                        "mesh '{name}' declares a GLB binary chunk that is missing"
                    )));
                }
            },
            gltf::buffer::Source::Uri(uri) => match uri
                .strip_prefix("data:")
                .and_then(|rest| rest.split_once(";base64,"))
            {
                Some((_, encoded)) => decode_base64(name, encoded)?,
                None => {
                    return Err(ChaosError::Asset(format!(
                        "mesh '{name}' references external buffer '{uri}' — only GLB or \
                         self-contained .gltf (base64 data URIs) are supported"
                    )));
                }
            },
        };
        buffers.push(data);
    }
    let Some(mesh) = document.meshes().next() else {
        return Err(ChaosError::Asset(format!("mesh '{name}' contains no mesh")));
    };
    let Some(primitive) = mesh
        .primitives()
        .find(|primitive| primitive.mode() == Mode::Triangles)
    else {
        return Err(ChaosError::Asset(format!(
            "mesh '{name}' has no TRIANGLES primitive"
        )));
    };

    let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(Vec::as_slice));
    let Some(position_reader) = reader.read_positions() else {
        return Err(ChaosError::Asset(format!(
            "mesh '{name}' has no readable positions (missing attribute or external buffer)"
        )));
    };
    let positions: Vec<[f32; 3]> = position_reader.collect();
    let uvs: Vec<[f32; 2]> = match reader.read_tex_coords(0) {
        Some(tex_coords) => tex_coords.into_f32().collect(),
        None => vec![[0.0, 0.0]; positions.len()],
    };
    if uvs.len() != positions.len() {
        return Err(ChaosError::Asset(format!(
            "mesh '{name}' has {} UVs for {} positions",
            uvs.len(),
            positions.len()
        )));
    }
    let indices: Vec<u32> = match reader.read_indices() {
        Some(index_reader) => index_reader.into_u32().collect(),
        None => (0..u32::try_from(positions.len()).unwrap_or(u32::MAX)).collect(),
    };
    Ok(MeshData {
        positions,
        uvs,
        indices,
    })
}

/// Décodeur base64 standard (RFC 4648, avec padding) — en std pur, pour les
/// buffers embarqués des `.gltf` auto-suffisants.
fn decode_base64(name: &str, encoded: &str) -> ChaosResult<Vec<u8>> {
    fn sextet(byte: u8) -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some(u32::from(byte - b'A')),
            b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let trimmed = encoded.trim_end_matches('=');
    let mut output = Vec::with_capacity(trimmed.len() * 3 / 4);
    let mut accumulator = 0u32;
    let mut bits = 0u32;
    for byte in trimmed.bytes() {
        let Some(value) = sextet(byte) else {
            return Err(ChaosError::Asset(format!(
                "mesh '{name}' has an invalid base64 buffer (byte {byte:#04x})"
            )));
        };
        accumulator = (accumulator << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((accumulator >> bits) as u8);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_glb(json: &str, bin: &[u8]) -> Vec<u8> {
        let mut json_bytes = json.as_bytes().to_vec();
        while !json_bytes.len().is_multiple_of(4) {
            json_bytes.push(b' ');
        }
        let mut bin_bytes = bin.to_vec();
        while !bin_bytes.len().is_multiple_of(4) {
            bin_bytes.push(0);
        }
        let bin_section = if bin.is_empty() {
            0
        } else {
            8 + bin_bytes.len()
        };
        let total = 12 + 8 + json_bytes.len() + bin_section;
        let mut glb = Vec::with_capacity(total);
        glb.extend_from_slice(&0x4654_6C67_u32.to_le_bytes());
        glb.extend_from_slice(&2_u32.to_le_bytes());
        glb.extend_from_slice(&u32::try_from(total).unwrap().to_le_bytes());
        glb.extend_from_slice(&u32::try_from(json_bytes.len()).unwrap().to_le_bytes());
        glb.extend_from_slice(&0x4E4F_534A_u32.to_le_bytes());
        glb.extend_from_slice(&json_bytes);
        if !bin.is_empty() {
            glb.extend_from_slice(&u32::try_from(bin_bytes.len()).unwrap().to_le_bytes());
            glb.extend_from_slice(&0x004E_4942_u32.to_le_bytes());
            glb.extend_from_slice(&bin_bytes);
        }
        glb
    }

    fn f32_bytes(values: &[f32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    fn u16_bytes(values: &[u16]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    /// Triangle complet : positions (36 o) + UV (24 o) + indices u16 (6 o).
    fn triangle_glb() -> Vec<u8> {
        let mut bin = f32_bytes(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
        bin.extend(f32_bytes(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]));
        bin.extend(u16_bytes(&[0, 1, 2]));
        let json = r#"{"asset":{"version":"2.0"},
            "buffers":[{"byteLength":66}],
            "bufferViews":[
                {"buffer":0,"byteOffset":0,"byteLength":36},
                {"buffer":0,"byteOffset":36,"byteLength":24},
                {"buffer":0,"byteOffset":60,"byteLength":6}],
            "accessors":[
                {"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]},
                {"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"},
                {"bufferView":2,"componentType":5123,"count":3,"type":"SCALAR"}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"indices":2}]}]}"#;
        build_glb(json, &bin)
    }

    fn positions_only_json(mode: Option<u32>) -> String {
        let mode = mode
            .map(|mode| format!(",\"mode\":{mode}"))
            .unwrap_or_default();
        format!(
            r#"{{"asset":{{"version":"2.0"}},
            "buffers":[{{"byteLength":36}}],
            "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":36}}],
            "accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}{mode}}}]}}]}}"#
        )
    }

    fn positions_bin() -> Vec<u8> {
        f32_bytes(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0])
    }

    #[test]
    fn glb_triangle_imports_positions_uvs_and_indices() {
        let ImportedAsset::Mesh(data) = GltfImporter.import("m", &triangle_glb()).unwrap() else {
            panic!("kind inattendu");
        };
        assert_eq!(
            data.positions,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        );
        assert_eq!(data.uvs, vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        assert_eq!(data.indices, vec![0, 1, 2]);
    }

    #[test]
    fn glb_without_uvs_fills_zeros() {
        let glb = build_glb(&positions_only_json(None), &positions_bin());
        let ImportedAsset::Mesh(data) = GltfImporter.import("m", &glb).unwrap() else {
            panic!("kind inattendu");
        };
        assert_eq!(data.uvs, vec![[0.0, 0.0]; 3]);
    }

    #[test]
    fn glb_without_indices_generates_the_sequence() {
        let glb = build_glb(&positions_only_json(None), &positions_bin());
        let ImportedAsset::Mesh(data) = GltfImporter.import("m", &glb).unwrap() else {
            panic!("kind inattendu");
        };
        assert_eq!(data.indices, vec![0, 1, 2]);
    }

    #[test]
    fn non_triangle_primitives_are_rejected() {
        let glb = build_glb(&positions_only_json(Some(0)), &positions_bin());
        let error = GltfImporter.import("m", &glb).unwrap_err();
        assert!(error.to_string().contains("TRIANGLES"));
    }

    #[test]
    fn external_buffers_are_rejected() {
        let json = r#"{"asset":{"version":"2.0"},
            "buffers":[{"uri":"model.bin","byteLength":36}],
            "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"#;
        let glb = build_glb(json, &[]);
        let error = GltfImporter.import("m", &glb).unwrap_err();
        assert!(error.to_string().contains("external buffer 'model.bin'"));
    }

    fn encode_base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let packed = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = u32::from(packed[0]) << 16 | u32::from(packed[1]) << 8 | u32::from(packed[2]);
            out.push(ALPHABET[(n >> 18) as usize & 63] as char);
            out.push(ALPHABET[(n >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[(n >> 6) as usize & 63] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[n as usize & 63] as char
            } else {
                '='
            });
        }
        out
    }

    #[test]
    fn self_contained_gltf_imports_via_base64_data_uri() {
        let encoded = encode_base64(&positions_bin());
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},
            "buffers":[{{"uri":"data:application/octet-stream;base64,{encoded}","byteLength":36}}],
            "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":36}}],
            "accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}}}]}}]}}"#
        );
        let ImportedAsset::Mesh(data) = GltfImporter.import("m", json.as_bytes()).unwrap() else {
            panic!("kind inattendu");
        };
        assert_eq!(
            data.positions,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        );
        assert_eq!(data.indices, vec![0, 1, 2]);
    }

    #[test]
    fn base64_decoder_matches_reference_vectors() {
        assert_eq!(decode_base64("m", "TWFu").unwrap(), b"Man");
        assert_eq!(decode_base64("m", "TWE=").unwrap(), b"Ma");
        assert_eq!(decode_base64("m", "TQ==").unwrap(), b"M");
        assert_eq!(decode_base64("m", "").unwrap(), b"");
        let error = decode_base64("m", "TW!u").unwrap_err();
        assert!(error.to_string().contains("base64"));
    }

    #[test]
    fn corrupt_bytes_are_a_named_error() {
        let error = GltfImporter
            .import("m", b"definitely not a glb")
            .unwrap_err();
        assert!(error.to_string().contains("not valid glTF"));
        assert!(error.to_string().contains('m'));
    }
}
