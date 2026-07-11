use chaos_core::{ChaosError, ChaosResult};

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
    /// Le texel brut, sans interpolation (pixel art, damiers nets).
    Nearest,
    /// L'interpolation bilinéaire — le standard.
    Linear,
}

/// Adressage hors [0, 1] : `Repeat` répète la texture (tiling),
/// `ClampToEdge` étire le bord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SamplerAddressMode {
    /// La texture se répète (tiling).
    Repeat,
    /// Le bord est étiré au-delà de [0, 1].
    ClampToEdge,
    /// La texture se répète en miroir (tiling sans couture visible).
    MirrorRepeat,
}

/// Description d'un sampler : COMMENT une texture est lue, indépendamment de
/// la texture elle-même — un même sampler sert autant de textures que voulu.
/// Les samplers de comparaison (ombres) étendront ce descripteur avec leur
/// besoin réel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SamplerDescriptor {
    /// Le label de diagnostic.
    pub label: String,
    /// Le filtrage d'échantillonnage (agrandissement et réduction).
    pub filter: SamplerFilter,
    /// Le filtrage ENTRE les niveaux de mips (`Linear` = trilinéaire).
    pub mip_filter: SamplerFilter,
    /// L'adressage hors [0, 1].
    pub address_mode: SamplerAddressMode,
    /// L'anisotropie maximale (1 = désactivée, 16 max) — au-delà de 1,
    /// tous les filtrages doivent être `Linear` (validé).
    pub anisotropy: u16,
}

impl SamplerDescriptor {
    /// Descripteur aux défauts du moteur : `Linear` + `Repeat`, mips en
    /// `Nearest`, anisotropie désactivée.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            filter: SamplerFilter::Linear,
            mip_filter: SamplerFilter::Nearest,
            address_mode: SamplerAddressMode::Repeat,
            anisotropy: 1,
        }
    }

    /// Fixe le filtrage.
    pub fn with_filter(mut self, filter: SamplerFilter) -> Self {
        self.filter = filter;
        self
    }

    /// Fixe le filtrage entre niveaux de mips.
    pub fn with_mip_filter(mut self, mip_filter: SamplerFilter) -> Self {
        self.mip_filter = mip_filter;
        self
    }

    /// Fixe l'adressage.
    pub fn with_address_mode(mut self, address_mode: SamplerAddressMode) -> Self {
        self.address_mode = address_mode;
        self
    }

    /// Fixe l'anisotropie maximale.
    pub fn with_anisotropy(mut self, anisotropy: u16) -> Self {
        self.anisotropy = anisotropy;
        self
    }

    /// Vérifie la cohérence du descripteur — appliquée par le Renderer
    /// AVANT le backend (la règle d'anisotropie est celle des API
    /// graphiques, vérifiée ici plutôt que découverte en erreur GPU).
    pub fn validate(&self) -> ChaosResult<()> {
        if self.anisotropy == 0 || self.anisotropy > 16 {
            return Err(ChaosError::Graphics(format!(
                "sampler '{}': anisotropy must be within 1..=16, got {}",
                self.label, self.anisotropy
            )));
        }
        if self.anisotropy > 1
            && (self.filter != SamplerFilter::Linear || self.mip_filter != SamplerFilter::Linear)
        {
            return Err(ChaosError::Graphics(format!(
                "sampler '{}': anisotropy above 1 requires Linear filtering everywhere",
                self.label
            )));
        }
        Ok(())
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
            .with_mip_filter(SamplerFilter::Linear)
            .with_address_mode(SamplerAddressMode::MirrorRepeat)
            .with_anisotropy(8);
        assert_eq!(descriptor.filter, SamplerFilter::Nearest);
        assert_eq!(descriptor.mip_filter, SamplerFilter::Linear);
        assert_eq!(descriptor.address_mode, SamplerAddressMode::MirrorRepeat);
        assert_eq!(descriptor.anisotropy, 8);
    }

    #[test]
    fn anisotropy_out_of_bounds_is_refused() {
        let zero = SamplerDescriptor::new("s").with_anisotropy(0);
        assert!(zero.validate().unwrap_err().to_string().contains("1..=16"));
        let over = SamplerDescriptor::new("s").with_anisotropy(32);
        assert!(over.validate().unwrap_err().to_string().contains("1..=16"));
    }

    #[test]
    fn anisotropy_requires_linear_filtering_everywhere() {
        let nearest_mips = SamplerDescriptor::new("s").with_anisotropy(4);
        assert!(
            nearest_mips
                .validate()
                .unwrap_err()
                .to_string()
                .contains("Linear filtering everywhere")
        );
        let trilinear = SamplerDescriptor::new("s")
            .with_mip_filter(SamplerFilter::Linear)
            .with_anisotropy(4);
        assert_eq!(trilinear.validate(), Ok(()));
    }
}
