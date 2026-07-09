/// Identifiant opaque d'un buffer GPU. Générationnel : un handle dont la
/// ressource a été détruite est détecté, jamais résolu vers un autre buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferHandle {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BufferKind {
    Vertex,
    Index,
}

/// Description d'un buffer GPU ; les données sont uploadées à la création
/// (buffers immutables pour l'instant — les mises à jour dynamiques
/// viendront avec leurs besoins réels).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferDescriptor {
    pub label: String,
    pub kind: BufferKind,
    pub contents: Vec<u8>,
}

impl BufferDescriptor {
    pub fn vertex(label: impl Into<String>, contents: Vec<u8>) -> Self {
        Self {
            label: label.into(),
            kind: BufferKind::Vertex,
            contents,
        }
    }

    pub fn index(label: impl Into<String>, contents: Vec<u8>) -> Self {
        Self {
            label: label.into(),
            kind: BufferKind::Index,
            contents,
        }
    }
}

/// Convertit des f32 en octets (endianness native, celle attendue par le GPU).
pub fn bytes_of_f32(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

/// Convertit des u16 en octets (endianness native) — indices de géométrie.
pub fn bytes_of_u16(values: &[u16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_of_f32_produces_native_bytes() {
        let bytes = bytes_of_f32(&[1.0, -2.5]);
        assert_eq!(bytes.len(), 8);
        assert_eq!(bytes[..4], 1.0f32.to_ne_bytes());
        assert_eq!(bytes[4..], (-2.5f32).to_ne_bytes());
    }

    #[test]
    fn descriptor_helpers_set_the_kind() {
        assert_eq!(
            BufferDescriptor::vertex("v", Vec::new()).kind,
            BufferKind::Vertex
        );
        assert_eq!(
            BufferDescriptor::index("i", Vec::new()).kind,
            BufferKind::Index
        );
    }
}
