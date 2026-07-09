use chaos_core::{Event, WindowEvent};
use log::{error, info, trace};

use crate::context::EngineContext;
use crate::subsystem::Subsystem;

/// Pilote de rendu : branche le service Renderer du contexte sur le cycle
/// de vie du moteur — resize, phase render, libération au shutdown.
/// Le Renderer lui-même appartient à l'EngineContext (créé par l'Engine).
pub(crate) struct RenderSubsystem;

impl Subsystem for RenderSubsystem {
    fn name(&self) -> &str {
        "renderer"
    }

    fn on_event(&mut self, event: &Event, context: &mut EngineContext) {
        if let Event::Window(WindowEvent::Resized { width, height }) = event
            && let Some(renderer) = context.renderer_mut()
        {
            renderer.resize(*width, *height);
        }
    }

    fn render(&mut self, context: &mut EngineContext) {
        let Some(renderer) = context.renderer_mut() else {
            return;
        };
        match renderer.render_frame() {
            Ok(outcome) => trace!("frame outcome: {outcome:?}"),
            Err(render_error) => {
                error!("frame rendering failed: {render_error}");
                context.request_exit();
            }
        }
    }

    fn shutdown(&mut self, context: &mut EngineContext) {
        if context.take_renderer().is_some() {
            info!("renderer released");
        }
    }
}
