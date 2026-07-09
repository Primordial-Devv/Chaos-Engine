use chaos_core::Color;

use crate::mesh::MeshHandle;
use crate::resources::{BufferHandle, PipelineHandle};

/// Ordre de dessin public : un pipeline + un mesh. Le renderer résout le
/// mesh en buffers au moment de construire le plan de frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawCommand {
    pub pipeline: PipelineHandle,
    pub mesh: MeshHandle,
}

/// Draw résolu du plan de frame — le vocabulaire buffers consommé par le
/// backend. `index_buffer` présent → rendu indexé (indices u16) ;
/// `element_count` compte les indices si indexé, les sommets sinon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameDraw {
    pub pipeline: PipelineHandle,
    pub vertex_buffer: Option<BufferHandle>,
    pub index_buffer: Option<BufferHandle>,
    pub element_count: u32,
}

/// Description de ce que le renderer doit produire pour une frame.
/// L'abstraction décrit le « quoi », le backend exécute le « comment ».
#[derive(Debug, Clone, PartialEq)]
pub struct FramePlan {
    pub clear_color: Color,
    pub draws: Vec<FrameDraw>,
}

/// Issue d'une frame : rendue, ou sautée pour une raison connue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    Rendered,
    Skipped(FrameSkipReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameSkipReason {
    SurfaceUnavailable,
    SurfaceReconfigured,
    ZeroArea,
}
