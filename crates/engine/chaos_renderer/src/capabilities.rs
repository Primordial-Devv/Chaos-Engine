//! Les CAPACITÉS du renderer : ce que le GPU et la plateforme OFFRENT,
//! ce que le renderer a DÉCIDÉ d'en faire, et POURQUOI — le rapport qui
//! rend les différences entre machines observables au lieu
//! d'implicites. Chaque capacité est détectée, utilisée quand elle
//! existe, remplacée par un fallback documenté ou coupée proprement
//! sinon ; les limites du device deviennent des REFUS nommés côté
//! Renderer, jamais des erreurs de validation backend. Le rapport est
//! STATIQUE (capturé à l'initialisation — le pendant des diagnostics
//! par frame) et s'affiche en lignes de log lisibles (`Display`).

use std::fmt;

/// Les LIMITES du device consommées par le renderer, en vocabulaire
/// Chaos. Les défauts sont les DÉFAUTS WebGPU — le plancher portable :
/// le renderer demande délibérément ces limites-là au device (jamais
/// les maximums de l'adaptateur — la robustesse avant la capacité,
/// l'élévation ciblée est l'extension notée), et les FAIT RESPECTER
/// avant le backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceLimits {
    /// Le côté maximal d'une texture 2D — et d'une FACE de cubemap
    /// (WebGPU ne les distingue pas). Les textures, cibles et shadow
    /// maps s'y mesurent.
    pub max_texture_2d: u32,
    /// La taille maximale d'un buffer GPU, en octets.
    pub max_buffer_bytes: u64,
    /// Le nombre maximal de bind groups par pipeline (le moteur en
    /// consomme 3 : frame, objet, material).
    pub max_bind_groups: u32,
    /// Les textures échantillonnées maximales par étage de shader (le
    /// PBR en consomme 7 : 5 material + environnement + ombre).
    pub max_sampled_textures_per_stage: u32,
    /// Les samplers maximaux par étage de shader.
    pub max_samplers_per_stage: u32,
    /// Les attachements couleur maximaux d'une passe (le moteur en
    /// utilise 1).
    pub max_color_attachments: u32,
    /// L'alignement minimal des offsets de buffers uniformes — RAPPORTÉ
    /// mais non consommé en V1 (les slots d'objets sont des buffers
    /// dédiés ; les dynamic offsets sont l'optimisation notée).
    pub uniform_offset_alignment: u32,
    /// L'anisotropie maximale des samplers (16 — le cœur WebGPU) ; la
    /// borne SOURCE de la validation des samplers.
    pub max_anisotropy: u16,
}

impl Default for DeviceLimits {
    fn default() -> Self {
        Self {
            max_texture_2d: 8192,
            max_buffer_bytes: 256 << 20,
            max_bind_groups: 4,
            max_sampled_textures_per_stage: 16,
            max_samplers_per_stage: 16,
            max_color_attachments: 8,
            uniform_offset_alignment: 256,
            max_anisotropy: 16,
        }
    }
}

/// Le STATUT décidé d'une capacité — jamais un état implicite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityStatus {
    /// Détectée et UTILISÉE.
    Active,
    /// Remplacée par un chemin de repli DOCUMENTÉ — la raison est nommée.
    Fallback {
        /// Pourquoi le repli.
        reason: String,
    },
    /// Option facultative COUPÉE proprement — la raison est nommée.
    Disabled {
        /// Pourquoi la coupure.
        reason: String,
    },
}

impl fmt::Display for CapabilityStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Active => write!(formatter, "active"),
            Self::Fallback { reason } => write!(formatter, "fallback ({reason})"),
            Self::Disabled { reason } => write!(formatter, "disabled ({reason})"),
        }
    }
}

/// UNE décision de capacité : le domaine, le statut choisi, et
/// l'EXPLICATION (ce qui a été détecté, pourquoi ce choix) — le contrat
/// « expliquer la décision prise ».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDecision {
    /// Le domaine de la capacité (« timestamp queries »,
    /// « presentation », « hdr », …).
    pub domain: String,
    /// Le statut décidé.
    pub status: CapabilityStatus,
    /// Ce qui a été détecté et le détail du choix.
    pub detail: String,
}

/// LE rapport des capacités du renderer : le backend et l'adaptateur
/// détectés, les limites RESPECTÉES, et une décision EXPLIQUÉE par
/// domaine — capturé une fois à l'initialisation
/// (`Renderer::capabilities`), inspectable sans UI (`Display`).
#[derive(Debug, Clone, PartialEq)]
pub struct RendererCapabilities {
    /// L'API graphique effective (« Metal », « Dx12 », « Vulkan »,
    /// « mock »).
    pub backend: String,
    /// L'adaptateur détecté (le nom du GPU).
    pub adapter: String,
    /// Les limites que le renderer fait respecter avant le backend.
    pub limits: DeviceLimits,
    /// Les décisions par domaine, dans l'ordre de la détection.
    pub decisions: Vec<CapabilityDecision>,
}

impl RendererCapabilities {
    /// La décision d'un domaine, si elle existe — l'inspection ciblée.
    pub fn decision(&self, domain: &str) -> Option<&CapabilityDecision> {
        self.decisions
            .iter()
            .find(|decision| decision.domain == domain)
    }
}

impl fmt::Display for RendererCapabilities {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            formatter,
            "capabilities: {} on {}",
            self.backend, self.adapter
        )?;
        let limits = &self.limits;
        writeln!(
            formatter,
            "  limits: textures {}px, buffers {} bytes, {} bind groups, {}/{} textures/samplers per stage, {} color attachments, uniform alignment {} B, anisotropy x{}",
            limits.max_texture_2d,
            limits.max_buffer_bytes,
            limits.max_bind_groups,
            limits.max_sampled_textures_per_stage,
            limits.max_samplers_per_stage,
            limits.max_color_attachments,
            limits.uniform_offset_alignment,
            limits.max_anisotropy
        )?;
        for (index, decision) in self.decisions.iter().enumerate() {
            if index > 0 {
                writeln!(formatter)?;
            }
            write!(
                formatter,
                "  {}: {} — {}",
                decision.domain, decision.status, decision.detail
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_webgpu_floor() {
        let limits = DeviceLimits::default();
        assert_eq!(limits.max_texture_2d, 8192);
        assert_eq!(limits.max_buffer_bytes, 256 << 20);
        assert_eq!(limits.max_bind_groups, 4);
        assert_eq!(limits.max_anisotropy, 16);
        assert_eq!(limits.uniform_offset_alignment, 256);
    }

    #[test]
    fn the_display_explains_every_decision() {
        let capabilities = RendererCapabilities {
            backend: String::from("Metal"),
            adapter: String::from("Apple M4 Pro"),
            limits: DeviceLimits::default(),
            decisions: vec![
                CapabilityDecision {
                    domain: String::from("timestamp queries"),
                    status: CapabilityStatus::Active,
                    detail: String::from("offered by the adapter"),
                },
                CapabilityDecision {
                    domain: String::from("surface format"),
                    status: CapabilityStatus::Fallback {
                        reason: String::from("no sRGB surface format offered"),
                    },
                    detail: String::from("using the first offered format"),
                },
                CapabilityDecision {
                    domain: String::from("gpu timing"),
                    status: CapabilityStatus::Disabled {
                        reason: String::from("not offered"),
                    },
                    detail: String::from("GPU time reported unavailable"),
                },
            ],
        };
        let text = capabilities.to_string();
        assert!(text.contains("Metal on Apple M4 Pro"));
        assert!(text.contains("textures 8192px"));
        assert!(text.contains("timestamp queries: active — offered by the adapter"));
        assert!(text.contains("fallback (no sRGB surface format offered)"));
        assert!(text.contains("disabled (not offered)"));
        assert_eq!(
            capabilities.decision("surface format").map(|d| &d.status),
            Some(&CapabilityStatus::Fallback {
                reason: String::from("no sRGB surface format offered")
            })
        );
        assert_eq!(capabilities.decision("unknown"), None);
    }
}
