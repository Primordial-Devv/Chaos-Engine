//! L'ÉCLAIRAGE de frame et ses réglages persistants : l'ambiante, les
//! lumières soumises puis collectées (filtrées, normalisées,
//! tronquées), l'environnement (cubemap, ciel, exposition) et les
//! réglages d'ombre directionnelle.

use super::*;

impl Renderer {
    /// Fixe la lumière AMBIANTE — un RÉGLAGE persistant (le patron de
    /// `set_clear_color`), jamais vidé par `clear_draws`. Le défaut est
    /// (noir, 0) : sans ambiante ni lumière, une surface éclairée est
    /// noire.
    pub fn set_ambient_light(&mut self, color: Color, intensity: f32) {
        self.ambient_color = color;
        self.ambient_intensity = intensity;
    }

    /// La lumière ambiante courante (couleur, intensité).
    pub fn ambient_light(&self) -> (Color, f32) {
        (self.ambient_color, self.ambient_intensity)
    }

    /// Fixe l'ENVIRONNEMENT de la scène — un RÉGLAGE persistant (le
    /// patron de l'ambiante), jamais vidé par `clear_draws`. La cubemap
    /// doit être vivante et de kind `Cube` ; l'intensité finie, positive
    /// ou nulle. Re-poser le MÊME cubemap ne rebinde pas le backend
    /// (mise à jour intensité/ciel seule). Backend d'abord, état
    /// ensuite : un refus laisse l'environnement précédent intact.
    pub fn set_environment(&mut self, descriptor: &EnvironmentDescriptor) -> ChaosResult<()> {
        let Some(info) = self.lifetime.texture_info(descriptor.cubemap) else {
            return Err(ChaosError::Graphics(String::from(
                "texture handle is stale or already destroyed",
            )));
        };
        if info.kind != TextureKind::Cube {
            return Err(ChaosError::Graphics(format!(
                "environment: texture '{}' is a {:?} texture — the environment expects a cubemap",
                info.label, info.kind
            )));
        }
        if !descriptor.intensity.is_finite() || descriptor.intensity < 0.0 {
            return Err(ChaosError::Graphics(format!(
                "environment '{}': intensity must be finite and non-negative, got {}",
                info.label, descriptor.intensity
            )));
        }
        if self.environment.as_ref().map(|env| env.cubemap) != Some(descriptor.cubemap) {
            self.backend.set_environment(Some(descriptor.cubemap))?;
        }
        self.environment = Some(EnvironmentState {
            cubemap: descriptor.cubemap,
            intensity: descriptor.intensity,
            sky: descriptor.sky,
        });
        Ok(())
    }

    /// Efface l'environnement : le backend rebinde son cube fallback
    /// noir (contribution nulle, sans branche shader), le ciel
    /// disparaît. Idempotent — effacer sans environnement est un no-op.
    pub fn clear_environment(&mut self) -> ChaosResult<()> {
        if self.environment.is_some() {
            self.backend.set_environment(None)?;
            self.environment = None;
        }
        Ok(())
    }

    /// L'inspection de l'environnement actif, s'il existe — le miroir
    /// lisible de l'état pour les outils.
    pub fn environment_info(&self) -> Option<EnvironmentInfo> {
        let environment = self.environment.as_ref()?;
        let info = self.lifetime.texture_info(environment.cubemap)?;
        Some(EnvironmentInfo {
            label: info.label.clone(),
            intensity: environment.intensity,
            sky: environment.sky,
            mip_levels: info.mip_levels,
        })
    }

    /// Configure les ombres de la lumière directionnelle principale — un
    /// réglage PERSISTANT (le patron de l'environnement), jamais vidé par
    /// `clear_draws`. La lumière qui projette est la PREMIÈRE
    /// directionnelle activée et valide de chaque frame ; sans
    /// directionnelle, la passe d'ombre est simplement absente. Le
    /// descripteur est VALIDÉ avant tout appel backend ; le backend n'est
    /// touché que si la RÉSOLUTION change (la map est recréée) — volume
    /// et biais sont des données par frame, réglables à chaud sans
    /// recréation.
    pub fn set_directional_shadow(
        &mut self,
        descriptor: &DirectionalShadowDescriptor,
    ) -> ChaosResult<()> {
        descriptor.validate()?;
        // La borne DEVICE, distincte de la borne engine (16..=8192 du
        // descripteur) : le message nomme LA limite qui refuse.
        let device_ceiling = self.capabilities.limits.max_texture_2d;
        if descriptor.resolution > device_ceiling {
            return Err(ChaosError::Graphics(format!(
                "shadow map resolution {} exceeds the device texture limit ({device_ceiling})",
                descriptor.resolution
            )));
        }
        if self.directional_shadow.map(|current| current.resolution) != Some(descriptor.resolution)
        {
            self.backend.set_shadow(Some(ShadowConfig {
                resolution: descriptor.resolution,
            }))?;
        }
        self.directional_shadow = Some(*descriptor);
        Ok(())
    }

    /// Efface les ombres : la shadow map backend est libérée, le
    /// fallback « tout éclairé » rebindé — plus aucune atténuation.
    /// Idempotent — effacer sans ombres est un no-op.
    pub fn clear_directional_shadow(&mut self) -> ChaosResult<()> {
        if self.directional_shadow.is_some() {
            self.backend.set_shadow(None)?;
            self.directional_shadow = None;
        }
        Ok(())
    }

    /// L'inspection des ombres configurées, s'il y en a — le miroir
    /// lisible de l'état pour les outils (résolution, volume, biais).
    pub fn directional_shadow_info(&self) -> Option<DirectionalShadowInfo> {
        self.directional_shadow
            .as_ref()
            .map(|settings| DirectionalShadowInfo {
                resolution: settings.resolution,
                volume: settings.volume,
                depth_bias: settings.depth_bias,
                normal_bias: settings.normal_bias,
            })
    }

    /// Fixe l'EXPOSITION globale, appliquée avant le tone mapping (les
    /// chemins tone-mappés : PBR et ciel — les modèles Unlit/Lit ne la
    /// lisent pas) — un réglage persistant, 1.0 par défaut. Refus
    /// explicite d'une valeur non finie ou non strictement positive.
    pub fn set_exposure(&mut self, exposure: f32) -> ChaosResult<()> {
        if !exposure.is_finite() || exposure <= 0.0 {
            return Err(ChaosError::Graphics(format!(
                "exposure must be a positive, finite value, got {exposure}"
            )));
        }
        self.exposure = exposure;
        Ok(())
    }

    /// L'exposition globale courante.
    pub fn exposure(&self) -> f32 {
        self.exposure
    }

    /// L'environnement de frame RÉSOLU pour le plan : l'intensité active
    /// (0 sans environnement) et l'exposition globale.
    pub(super) fn frame_environment(&self) -> FrameEnvironment {
        FrameEnvironment {
            intensity: self.environment.as_ref().map_or(0.0, |env| env.intensity),
            exposure: self.exposure,
        }
    }

    /// Soumet une lumière pour la frame de simulation courante — le
    /// pendant lumineux de `queue_draw` : re-soumise chaque frame, vidée
    /// par `clear_draws`. Une lumière INVALIDE (direction nulle,
    /// intensité négative, cône dégénéré, NaN) est écartée ici avec un
    /// warn — jamais envoyée au GPU. Au-delà de [`MAX_LIGHTS`] lumières
    /// activées, les premières soumises gagnent (troncature prévisible,
    /// un warn par épisode).
    pub fn submit_light(&mut self, light: Light) {
        if let Some(reason) = light.invalid_reason() {
            warn!("light dropped: {reason}");
            return;
        }
        self.frame_lights.push(light);
    }

    /// La COLLECTION d'éclairage de la frame — le chemin partagé de
    /// `render_frame` et `render_to_target` : filtre les lumières
    /// désactivées, normalise les directions, tronque à [`MAX_LIGHTS`]
    /// (les premières soumises gagnent) avec un warn par épisode de
    /// dépassement.
    pub(super) fn collect_lights(&mut self) -> FrameLights {
        let enabled = self.frame_lights.iter().filter(|light| light.is_enabled());
        let lights: Vec<Light> = enabled
            .clone()
            .take(MAX_LIGHTS)
            .map(Light::normalized)
            .collect();
        let submitted = enabled.count();
        if submitted > MAX_LIGHTS {
            if !self.lights_truncation_warned {
                warn!(
                    "light overflow: {submitted} enabled lights submitted, only the first {MAX_LIGHTS} are kept"
                );
                self.lights_truncation_warned = true;
            }
        } else {
            self.lights_truncation_warned = false;
        }
        FrameLights {
            ambient_color: self.ambient_color,
            ambient_intensity: self.ambient_intensity,
            lights,
        }
    }
}
