/// Identifiant opaque d'un sampler GPU. Générationnel : un handle dont la
/// ressource a été détruite est détecté, jamais résolu vers un autre sampler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplerHandle {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

/// Filtrage d'échantillonnage : `Nearest` = texel brut (pixel art, damiers),
/// `Linear` = interpolation bilinéaire (le standard).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SamplerFilter {
    Nearest,
    Linear,
}

/// Adressage hors [0, 1] : `Repeat` répète la texture (tiling),
/// `ClampToEdge` étire le bord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SamplerAddressMode {
    Repeat,
    ClampToEdge,
}

/// Description d'un sampler : COMMENT une texture est lue, indépendamment de
/// la texture elle-même — un même sampler sert autant de textures que voulu.
/// L'anisotropie et les samplers de comparaison (ombres) étendront ce
/// descripteur avec leurs besoins réels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SamplerDescriptor {
    pub label: String,
    pub filter: SamplerFilter,
    pub address_mode: SamplerAddressMode,
}

impl SamplerDescriptor {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            filter: SamplerFilter::Linear,
            address_mode: SamplerAddressMode::Repeat,
        }
    }

    pub fn with_filter(mut self, filter: SamplerFilter) -> Self {
        self.filter = filter;
        self
    }

    pub fn with_address_mode(mut self, address_mode: SamplerAddressMode) -> Self {
        self.address_mode = address_mode;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_linear_and_repeat() {
        let descriptor = SamplerDescriptor::new("s");
        assert_eq!(descriptor.label, "s");
        assert_eq!(descriptor.filter, SamplerFilter::Linear);
        assert_eq!(descriptor.address_mode, SamplerAddressMode::Repeat);
    }

    #[test]
    fn builders_override_the_defaults() {
        let descriptor = SamplerDescriptor::new("s")
            .with_filter(SamplerFilter::Nearest)
            .with_address_mode(SamplerAddressMode::ClampToEdge);
        assert_eq!(descriptor.filter, SamplerFilter::Nearest);
        assert_eq!(descriptor.address_mode, SamplerAddressMode::ClampToEdge);
    }
}
