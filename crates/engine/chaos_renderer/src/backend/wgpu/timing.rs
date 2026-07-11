use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// Le slot est libre — prêt à recevoir une nouvelle mesure.
const FREE: u8 = 0;
/// Le mapping est demandé — la mesure est en vol.
const PENDING: u8 = 1;
/// Le mapping a abouti — la mesure attend d'être lue.
const READY: u8 = 2;

/// Deux timestamps par frame : le début (première passe exécutée) et la
/// fin (dernière passe) — le span GPU de la frame entière.
const QUERY_COUNT: u32 = 2;
const RESULT_BYTES: u64 = 16;

/// Le CHRONOMÈTRE GPU du backend : de VRAIES timestamp queries wgpu —
/// résolues vers un ring de readbacks mappés en asynchrone, JAMAIS
/// bloquant (`PollType::Poll`). La mesure disponible est celle de la
/// dernière frame RÉSOLUE (quelques frames de latence — documenté) ;
/// ring saturé → la mesure de la frame est SAUTÉE, jamais inventée.
pub(super) struct GpuTimer {
    query_set: wgpu::QuerySet,
    period: f32,
    resolve_buffer: wgpu::Buffer,
    readbacks: Vec<Readback>,
    latest_ms: Option<f32>,
}

struct Readback {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

impl GpuTimer {
    pub(super) fn new(device: &wgpu::Device, period: f32) -> Self {
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("chaos.gpu_timer"),
            ty: wgpu::QueryType::Timestamp,
            count: QUERY_COUNT,
        });
        let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chaos.gpu_timer.resolve"),
            size: RESULT_BYTES,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readbacks = (0..3)
            .map(|_| Readback {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("chaos.gpu_timer.readback"),
                    size: RESULT_BYTES,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(FREE)),
            })
            .collect();
        Self {
            query_set,
            period,
            resolve_buffer,
            readbacks,
            latest_ms: None,
        }
    }

    /// L'écriture de timestamps d'UNE passe : le début (index 0) et/ou
    /// la fin (index 1) du span de la frame — `None` si la passe ne
    /// borne rien.
    pub(super) fn pass_writes(
        &self,
        begin: bool,
        end: bool,
    ) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        if !begin && !end {
            return None;
        }
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: begin.then_some(0),
            end_of_pass_write_index: end.then_some(1),
        })
    }

    /// Après le DERNIER submit chronométré de la frame : résout les deux
    /// timestamps vers un slot LIBRE du ring et demande son mapping —
    /// ring saturé → la mesure est sautée.
    pub(super) fn finish_frame(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let Some(slot) = self
            .readbacks
            .iter()
            .find(|slot| slot.state.load(Ordering::Acquire) == FREE)
        else {
            return;
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("chaos.gpu_timer"),
        });
        encoder.resolve_query_set(&self.query_set, 0..QUERY_COUNT, &self.resolve_buffer, 0);
        encoder.copy_buffer_to_buffer(&self.resolve_buffer, 0, &slot.buffer, 0, RESULT_BYTES);
        queue.submit(std::iter::once(encoder.finish()));
        slot.state.store(PENDING, Ordering::Release);
        let state = Arc::clone(&slot.state);
        slot.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                state.store(if result.is_ok() { READY } else { FREE }, Ordering::Release);
            });
    }

    /// Avance les mappings (poll NON bloquant) et récolte les mesures
    /// prêtes — la dernière résolue devient `latest_ms`.
    pub(super) fn poll(&mut self, device: &wgpu::Device) {
        let _ = device.poll(wgpu::PollType::Poll);
        for slot in &self.readbacks {
            if slot.state.load(Ordering::Acquire) != READY {
                continue;
            }
            if let Ok(mapped) = slot.buffer.slice(..).get_mapped_range() {
                let begin = u64::from_le_bytes(mapped[0..8].try_into().unwrap_or_default());
                let end = u64::from_le_bytes(mapped[8..16].try_into().unwrap_or_default());
                // Un couple invalide (compteur réinitialisé, zéros) est
                // ÉCARTÉ — jamais une valeur inventée.
                if end > begin {
                    let nanoseconds = (end - begin) as f32 * self.period;
                    self.latest_ms = Some(nanoseconds / 1_000_000.0);
                }
            }
            slot.buffer.unmap();
            slot.state.store(FREE, Ordering::Release);
        }
    }

    /// La dernière mesure RÉSOLUE, en millisecondes — `None` tant que
    /// rien n'est revenu du GPU.
    pub(super) fn latest_ms(&self) -> Option<f32> {
        self.latest_ms
    }
}
