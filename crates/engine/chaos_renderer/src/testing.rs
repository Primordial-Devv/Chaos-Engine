//! Le BANC D'ESSAI du renderer : le backend factice à JOURNAL — chaque
//! appel backend devient une ligne assertable, les suffixes n'existent
//! que HORS défaut (la stabilité des assertions exactes historiques).
//! Partagé entre les tests unitaires (`renderer::tests`) et la suite de
//! stress & régression (`suite`). L'outcome est COMMUTABLE en cours de
//! run (`OutcomeSwitch`) — les pertes de surface et les erreurs backend
//! se scénarisent.

use std::sync::{Arc, Mutex};

use chaos_core::math::{Mat4, Vec3};
use chaos_core::{ChaosError, ChaosResult, Color};

use crate::backend::GraphicsBackend;
use crate::capabilities::{
    CapabilityDecision, CapabilityStatus, DeviceLimits, RendererCapabilities,
};
use crate::diagnostics::GpuTiming;
use crate::frame::{FrameDraw, FrameEnvironment, FrameOutcome, FramePlan, RenderDestination};
use crate::light::Light;
use crate::pass::PassLoad;
use crate::renderer::Renderer;
use crate::resources::{
    BufferDescriptor, BufferHandle, DepthCompare, MaterialBindingDescriptor, MaterialBindingHandle,
    MaterialParams, PipelineDescriptor, PipelineHandle, PrimitiveTopology, RenderTargetDescriptor,
    RenderTargetHandle, SamplerDescriptor, SamplerFilter, SamplerHandle, ShaderSource,
    TextureDescriptor, TextureHandle, TextureKind,
};
use crate::shadow::ShadowConfig;

/// Le journal du backend factice — chaque appel backend, une ligne.
#[derive(Clone, Default)]
pub(crate) struct Journal(Arc<Mutex<Vec<String>>>);

impl Journal {
    pub(crate) fn push(&self, entry: String) {
        self.0.lock().unwrap().push(entry);
    }

    pub(crate) fn entries(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// L'issue COMMUTABLE du prochain `render` du mock : un outcome, ou une
/// erreur backend injectée — la robustesse se scénarise en cours de run.
#[derive(Clone)]
pub(crate) struct OutcomeSwitch(Arc<Mutex<Result<FrameOutcome, String>>>);

impl OutcomeSwitch {
    fn new(outcome: FrameOutcome) -> Self {
        Self(Arc::new(Mutex::new(Ok(outcome))))
    }

    /// Les prochains `render` rendent cet outcome.
    pub(crate) fn set(&self, outcome: FrameOutcome) {
        *self.0.lock().unwrap() = Ok(outcome);
    }

    /// Le prochain `render` ÉCHOUE avec cette raison (jusqu'au prochain
    /// `set`) — l'erreur backend fatale, injectée.
    pub(crate) fn fail(&self, reason: &str) {
        *self.0.lock().unwrap() = Err(String::from(reason));
    }

    fn current(&self) -> Result<FrameOutcome, String> {
        self.0.lock().unwrap().clone()
    }
}

// Les suffixes de paramètres material, émis SEULEMENT hors défaut
// (MaterialParams::default() fait foi) — la stabilité des assertions.
fn push_param_suffixes(entry: &mut String, params: &MaterialParams) {
    let defaults = MaterialParams::default();
    if params.metallic != defaults.metallic {
        entry.push_str(&format!(" metallic={}", params.metallic));
    }
    if params.roughness != defaults.roughness {
        entry.push_str(&format!(" roughness={}", params.roughness));
    }
    if params.emissive != defaults.emissive {
        entry.push_str(&format!(
            " emissive=({}, {}, {})",
            params.emissive.r, params.emissive.g, params.emissive.b
        ));
    }
    if params.receive_shadows != defaults.receive_shadows {
        entry.push_str(" recv=off");
    }
    if params.alpha_cutoff != defaults.alpha_cutoff {
        entry.push_str(&format!(" cutoff={}", params.alpha_cutoff));
    }
}

// Le format d'UN draw dans les lignes du journal — partagé par les
// passes et l'ombre ; le suffixe ` inst=N` n'apparaît que sur les
// draws INSTANCIÉS (la stabilité des assertions historiques).
fn draw_piece(draw: &FrameDraw) -> String {
    let model = draw.model.w_axis;
    let mut piece = format!(
        "({}, {:?}, {:?}, {}, b={:?}, m=[{}, {}, {}]",
        draw.pipeline.0,
        draw.vertex_buffer.map(|buffer| buffer.index),
        draw.index_buffer.map(|buffer| buffer.index),
        draw.element_count,
        draw.binding.map(|binding| binding.index),
        model.x,
        model.y,
        model.z
    );
    if let Some(range) = draw.instances {
        piece.push_str(&format!(", inst={}", range.count));
    }
    piece.push(')');
    piece
}

struct MockBackend {
    journal: Journal,
    outcome: OutcomeSwitch,
    limits: DeviceLimits,
    pipelines_created: u32,
    buffers_created: u32,
    textures_created: u32,
    samplers_created: u32,
    material_bindings_created: u32,
    render_targets_created: u32,
}

impl GraphicsBackend for MockBackend {
    fn description(&self) -> String {
        String::from("mock backend")
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.journal.push(format!("resize {width}x{height}"));
    }

    fn create_pipeline(
        &mut self,
        descriptor: &PipelineDescriptor,
        shader: &ShaderSource,
    ) -> ChaosResult<PipelineHandle> {
        let ShaderSource::Wgsl(code) = shader;
        let mut entry = format!(
            "create_pipeline {} code={}",
            descriptor.label,
            &code[..code.len().min(24)]
        );
        if let Some(format) = descriptor.color_target {
            entry.push_str(&format!(" target={format:?}"));
        }
        if descriptor.transparent {
            entry.push_str(" blend=alpha");
        }
        match descriptor.depth_compare {
            DepthCompare::Less => {}
            DepthCompare::LessEqual => entry.push_str(" depth=less_equal"),
            DepthCompare::Always => entry.push_str(" depth=always"),
        }
        if descriptor.topology == PrimitiveTopology::LineList {
            entry.push_str(" topology=lines");
        }
        if descriptor.depth_only {
            entry.push_str(" depth_only");
        }
        if descriptor.fragment_entry != "fs_main" {
            entry.push_str(&format!(" entry={}", descriptor.fragment_entry));
        }
        if descriptor.instance_layout.is_some() {
            entry.push_str(" instanced");
        }
        self.journal.push(entry);
        let handle = PipelineHandle(self.pipelines_created);
        self.pipelines_created += 1;
        Ok(handle)
    }

    fn create_buffer(&mut self, descriptor: &BufferDescriptor) -> ChaosResult<BufferHandle> {
        self.journal.push(format!(
            "create_buffer {} kind={:?} bytes={}",
            descriptor.label,
            descriptor.kind,
            descriptor.contents.len()
        ));
        let handle = BufferHandle {
            index: self.buffers_created,
            generation: 0,
        };
        self.buffers_created += 1;
        Ok(handle)
    }

    fn destroy_buffer(&mut self, handle: BufferHandle) -> ChaosResult<()> {
        self.journal
            .push(format!("destroy_buffer index={}", handle.index));
        Ok(())
    }

    fn create_texture(&mut self, descriptor: &TextureDescriptor) -> ChaosResult<TextureHandle> {
        let mut entry = format!(
            "create_texture {} {}x{} format={:?} usage={:?} bytes={}",
            descriptor.label,
            descriptor.width,
            descriptor.height,
            descriptor.format,
            descriptor.usage,
            descriptor.pixels.len()
        );
        if descriptor.kind != TextureKind::D2 {
            entry.push_str(&format!(" kind={:?}", descriptor.kind));
        }
        if descriptor.mip_level_count() > 1 {
            entry.push_str(&format!(" levels={}", descriptor.mip_level_count()));
        }
        self.journal.push(entry);
        let handle = TextureHandle {
            index: self.textures_created,
            generation: 0,
        };
        self.textures_created += 1;
        Ok(handle)
    }

    fn update_texture(&mut self, handle: TextureHandle, pixels: &[u8]) -> ChaosResult<()> {
        self.journal.push(format!(
            "update_texture index={} bytes={}",
            handle.index,
            pixels.len()
        ));
        Ok(())
    }

    fn destroy_texture(&mut self, handle: TextureHandle) -> ChaosResult<()> {
        self.journal
            .push(format!("destroy_texture index={}", handle.index));
        Ok(())
    }

    fn create_sampler(&mut self, descriptor: &SamplerDescriptor) -> ChaosResult<SamplerHandle> {
        let mut entry = format!(
            "create_sampler {} filter={:?} address={:?}",
            descriptor.label, descriptor.filter, descriptor.address_mode
        );
        if descriptor.mip_filter != SamplerFilter::Nearest {
            entry.push_str(&format!(" mips={:?}", descriptor.mip_filter));
        }
        if descriptor.anisotropy > 1 {
            entry.push_str(&format!(" aniso={}", descriptor.anisotropy));
        }
        self.journal.push(entry);
        let handle = SamplerHandle {
            index: self.samplers_created,
            generation: 0,
        };
        self.samplers_created += 1;
        Ok(handle)
    }

    fn destroy_sampler(&mut self, handle: SamplerHandle) -> ChaosResult<()> {
        self.journal
            .push(format!("destroy_sampler index={}", handle.index));
        Ok(())
    }

    fn create_material_binding(
        &mut self,
        descriptor: &MaterialBindingDescriptor,
    ) -> ChaosResult<MaterialBindingHandle> {
        let mut entry = format!(
            "create_material_binding {} texture={} sampler={} color=({}, {}, {}, {}) mr={} normal={} ao={} em={}",
            descriptor.label,
            descriptor.texture.index,
            descriptor.sampler.index,
            descriptor.params.base_color.r,
            descriptor.params.base_color.g,
            descriptor.params.base_color.b,
            descriptor.params.base_color.a,
            descriptor.metallic_roughness_texture.index,
            descriptor.normal_map.index,
            descriptor.occlusion_texture.index,
            descriptor.emissive_texture.index
        );
        push_param_suffixes(&mut entry, &descriptor.params);
        self.journal.push(entry);
        let handle = MaterialBindingHandle {
            index: self.material_bindings_created,
            generation: 0,
        };
        self.material_bindings_created += 1;
        Ok(handle)
    }

    fn update_material_binding(
        &mut self,
        handle: MaterialBindingHandle,
        params: &MaterialParams,
    ) -> ChaosResult<()> {
        let mut entry = format!(
            "update_material_binding index={} color=({}, {}, {}, {})",
            handle.index,
            params.base_color.r,
            params.base_color.g,
            params.base_color.b,
            params.base_color.a
        );
        push_param_suffixes(&mut entry, params);
        self.journal.push(entry);
        Ok(())
    }

    fn destroy_material_binding(&mut self, handle: MaterialBindingHandle) -> ChaosResult<()> {
        self.journal
            .push(format!("destroy_material_binding index={}", handle.index));
        Ok(())
    }

    fn set_environment(&mut self, cubemap: Option<TextureHandle>) -> ChaosResult<()> {
        match cubemap {
            Some(handle) => self
                .journal
                .push(format!("set_environment index={}", handle.index)),
            None => self.journal.push(String::from("set_environment none")),
        }
        Ok(())
    }

    fn set_shadow(&mut self, config: Option<ShadowConfig>) -> ChaosResult<()> {
        match config {
            Some(config) => self
                .journal
                .push(format!("set_shadow resolution={}", config.resolution)),
            None => self.journal.push(String::from("set_shadow none")),
        }
        Ok(())
    }

    fn render(&mut self, plan: &FramePlan) -> ChaosResult<FrameOutcome> {
        // L'issue commutée se lit d'ABORD : une erreur injectée remonte
        // sans journaliser — le backend fatal n'a rien exécuté.
        let outcome = match self.outcome.current() {
            Err(reason) => return Err(ChaosError::Graphics(reason)),
            Ok(outcome) => outcome,
        };
        // La ligne `lights` n'apparaît que HORS défaut (aucune
        // lumière, ambiante nulle) — les assertions exactes des
        // frames non éclairées restent stables.
        if plan.lights.is_lit() {
            let mut entry = format!(
                "lights ambient=({}, {}, {}, {}) count={}",
                plan.lights.ambient_color.r,
                plan.lights.ambient_color.g,
                plan.lights.ambient_color.b,
                plan.lights.ambient_intensity,
                plan.lights.lights.len()
            );
            for light in &plan.lights.lights {
                let piece = match light {
                    Light::Directional {
                        direction,
                        color,
                        intensity,
                        ..
                    } => format!(
                        " [directional d=({}, {}, {}) c=({}, {}, {}) i={}]",
                        direction.x, direction.y, direction.z, color.r, color.g, color.b, intensity
                    ),
                    Light::Point {
                        position,
                        color,
                        intensity,
                        range,
                        ..
                    } => format!(
                        " [point p=({}, {}, {}) r={} c=({}, {}, {}) i={}]",
                        position.x,
                        position.y,
                        position.z,
                        range,
                        color.r,
                        color.g,
                        color.b,
                        intensity
                    ),
                    Light::Spot {
                        position,
                        direction,
                        range,
                        ..
                    } => format!(
                        " [spot p=({}, {}, {}) d=({}, {}, {}) r={}]",
                        position.x,
                        position.y,
                        position.z,
                        direction.x,
                        direction.y,
                        direction.z,
                        range
                    ),
                };
                entry.push_str(&piece);
            }
            self.journal.push(entry);
        }
        // La ligne `environment` n'apparaît que HORS défaut (pas
        // d'environnement, exposition 1) — même contrat de stabilité
        // que la ligne `lights`.
        if plan.environment != FrameEnvironment::default() {
            self.journal.push(format!(
                "environment intensity={} exposure={}",
                plan.environment.intensity, plan.environment.exposure
            ));
        }
        // La ligne `shadow` n'apparaît que si le plan porte une passe
        // d'ombre — les assertions exactes historiques restent
        // stables. Elle PRÉCÈDE les lignes de passes : la passe
        // d'ombre s'exécute avant toutes.
        if let Some(shadow) = &plan.shadow {
            let vp = shadow.view_projection.w_axis;
            let draws: Vec<String> = shadow.draws.iter().map(draw_piece).collect();
            self.journal.push(format!(
                "shadow vp=[{}, {}, {}] res={} light={} draws=[{}]",
                vp.x,
                vp.y,
                vp.z,
                shadow.resolution,
                shadow.light_index,
                draws.join(", ")
            ));
        }
        // Une ligne PAR PASSE, au format historique — les suffixes
        // (dest, pass, load) n'apparaissent que hors défaut, dans
        // cet ordre, pour la stabilité des assertions existantes.
        for pass in &plan.passes {
            let (color, keep) = match pass.load {
                PassLoad::Clear(color) => (color, false),
                PassLoad::Keep => (Color::BLACK, true),
            };
            let vp = pass.view_projection.w_axis;
            let draws: Vec<String> = pass.draws.iter().map(draw_piece).collect();
            let mut entry = format!(
                "render r={} g={} b={} a={} vp=[{}, {}, {}] draws=[{}]",
                color.r,
                color.g,
                color.b,
                color.a,
                vp.x,
                vp.y,
                vp.z,
                draws.join(", ")
            );
            if let RenderDestination::Target(target) = pass.destination {
                entry.push_str(&format!(" dest=target{}", target.index));
            }
            if pass.label != "chaos.main" {
                entry.push_str(&format!(" pass={}", pass.label));
            }
            if keep {
                entry.push_str(" load=keep");
            }
            if pass.camera_position != Vec3::ZERO {
                entry.push_str(&format!(
                    " cam=({}, {}, {})",
                    pass.camera_position.x, pass.camera_position.y, pass.camera_position.z
                ));
            }
            // Le suffixe DEBUG n'apparaît que si la passe porte des
            // batches — les assertions exactes historiques restent
            // stables.
            if !pass.debug.is_empty() {
                let batches: Vec<String> = pass
                    .debug
                    .iter()
                    .map(|batch| format!("v={} p={}", batch.vertex_count, batch.pipeline.0))
                    .collect();
                entry.push_str(&format!(" dbg=[{}]", batches.join(", ")));
            }
            self.journal.push(entry);
        }
        Ok(outcome)
    }

    fn create_render_target(
        &mut self,
        descriptor: &RenderTargetDescriptor,
    ) -> ChaosResult<(RenderTargetHandle, TextureHandle)> {
        self.journal.push(format!(
            "create_render_target {} {}x{} format={:?}",
            descriptor.label, descriptor.width, descriptor.height, descriptor.format
        ));
        let color = TextureHandle {
            index: self.textures_created,
            generation: 0,
        };
        self.textures_created += 1;
        let handle = RenderTargetHandle {
            index: self.render_targets_created,
            generation: 0,
        };
        self.render_targets_created += 1;
        Ok((handle, color))
    }

    fn destroy_render_target(&mut self, handle: RenderTargetHandle) -> ChaosResult<()> {
        self.journal
            .push(format!("destroy_render_target index={}", handle.index));
        Ok(())
    }

    // Le mock ne mesure RIEN : il le DIT — jamais un zéro inventé.
    fn gpu_frame_time(&self) -> GpuTiming {
        GpuTiming::Unavailable {
            reason: String::from("the mock backend does not measure GPU time"),
        }
    }

    // Le rapport DÉTERMINISTE du banc d'essai : les défauts WebGPU
    // (abaissables par test — les refus device), les décisions
    // explicites, AUCUNE ligne de journal (les assertions exactes
    // historiques survivent).
    fn capabilities(&self) -> RendererCapabilities {
        RendererCapabilities {
            backend: String::from("mock"),
            adapter: String::from("journal"),
            limits: self.limits,
            decisions: vec![
                CapabilityDecision {
                    domain: String::from("timestamp queries"),
                    status: CapabilityStatus::Disabled {
                        reason: String::from("the mock backend does not measure GPU time"),
                    },
                    detail: String::from("GPU time is reported unavailable, never invented"),
                },
                CapabilityDecision {
                    domain: String::from("presentation"),
                    status: CapabilityStatus::Active,
                    detail: String::from("deterministic mock presentation"),
                },
            ],
        }
    }
}

fn build(outcome: OutcomeSwitch, limits: DeviceLimits) -> (Renderer, Journal) {
    let journal = Journal::default();
    let renderer = Renderer::with_backend(Box::new(MockBackend {
        journal: journal.clone(),
        outcome,
        limits,
        pipelines_created: 0,
        buffers_created: 0,
        textures_created: 0,
        samplers_created: 0,
        material_bindings_created: 0,
        render_targets_created: 0,
    }));
    (renderer, journal)
}

/// La caméra LARGE du banc d'essai : une échelle pure — tout ce qui vit
/// dans ±1000 (z arrière compris) est VISIBLE, et le `w_axis` reste
/// nul : les assertions exactes historiques (`vp=[0, 0, 0]`) survivent
/// telles quelles. La caméra par défaut du moteur (identité) ne voit
/// que le cube NDC — un consommateur déclare la sienne (la démo, à
/// chaque update).
fn set_wide_bench_camera(renderer: &mut Renderer) {
    renderer.set_view_projection(Mat4::from_scale(Vec3::new(0.001, 0.001, -0.001)));
}

pub(crate) fn mock_renderer_with_limits(limits: DeviceLimits) -> (Renderer, Journal) {
    build(OutcomeSwitch::new(FrameOutcome::Rendered), limits)
}

pub(crate) fn mock_renderer_with(outcome: FrameOutcome) -> (Renderer, Journal) {
    build(OutcomeSwitch::new(outcome), DeviceLimits::default())
}

pub(crate) fn mock_renderer() -> (Renderer, Journal) {
    let (mut renderer, journal) = mock_renderer_with(FrameOutcome::Rendered);
    set_wide_bench_camera(&mut renderer);
    (renderer, journal)
}

/// Le banc à issue COMMUTABLE : la caméra large posée, l'interrupteur
/// rendu — la suite scénarise pertes de surface et erreurs backend.
pub(crate) fn mock_renderer_switchable() -> (Renderer, Journal, OutcomeSwitch) {
    let switch = OutcomeSwitch::new(FrameOutcome::Rendered);
    let (mut renderer, journal) = build(switch.clone(), DeviceLimits::default());
    set_wide_bench_camera(&mut renderer);
    (renderer, journal, switch)
}

pub(crate) fn render_lines(journal: &Journal) -> Vec<String> {
    journal
        .entries()
        .into_iter()
        .filter(|entry| entry.starts_with("render "))
        .collect()
}

pub(crate) fn create_pipeline_lines(journal: &Journal) -> Vec<String> {
    journal
        .entries()
        .into_iter()
        .filter(|entry| entry.starts_with("create_pipeline"))
        .collect()
}

pub(crate) fn shadow_lines(journal: &Journal) -> Vec<String> {
    journal
        .entries()
        .into_iter()
        .filter(|entry| entry.starts_with("shadow "))
        .collect()
}

pub(crate) fn set_shadow_lines(journal: &Journal) -> Vec<String> {
    journal
        .entries()
        .into_iter()
        .filter(|entry| entry.starts_with("set_shadow"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::FrameSkipReason;

    #[test]
    fn the_outcome_switch_drives_the_mock() {
        // Le banc se prouve lui-même : l'issue commute EN COURS de run,
        // l'erreur injectée remonte de `render_frame`, et la frame
        // suivante est saine — l'état jamais empoisonné.
        let (mut renderer, journal, switch) = mock_renderer_switchable();
        assert_eq!(renderer.render_frame().unwrap(), FrameOutcome::Rendered);
        switch.set(FrameOutcome::Skipped(FrameSkipReason::SurfaceReconfigured));
        assert_eq!(
            renderer.render_frame().unwrap(),
            FrameOutcome::Skipped(FrameSkipReason::SurfaceReconfigured)
        );
        switch.fail("injected backend failure");
        let failure = renderer.render_frame().unwrap_err();
        assert!(failure.to_string().contains("injected backend failure"));
        switch.set(FrameOutcome::Rendered);
        assert_eq!(renderer.render_frame().unwrap(), FrameOutcome::Rendered);
        // La frame en échec n'a RIEN journalisé — le backend fatal n'a
        // rien exécuté.
        assert_eq!(render_lines(&journal).len(), 3);
    }
}
