use chaos_core::{ChaosResult, Color, Event, WindowEvent};
use chaos_renderer::Renderer;
use chaos_window::WindowHandle;
use log::{error, info};

use crate::context::EngineContext;
use crate::subsystem::Subsystem;

/// Adaptateur qui branche le renderer sur le cycle de vie du moteur :
/// init = attache GPU, Resized = resize, render = frame, shutdown = libération.
pub(crate) struct RenderSubsystem {
    window: WindowHandle,
    clear_color: Color,
    renderer: Option<Renderer>,
}

impl RenderSubsystem {
    pub(crate) fn new(window: WindowHandle, clear_color: Color) -> Self {
        Self {
            window,
            clear_color,
            renderer: None,
        }
    }
}

impl Subsystem for RenderSubsystem {
    fn name(&self) -> &str {
        "renderer"
    }

    fn init(&mut self, _context: &mut EngineContext) -> ChaosResult<()> {
        let (width, height) = self.window.inner_size();
        let mut renderer = Renderer::attach(self.window.clone(), width, height)?;
        renderer.set_clear_color(self.clear_color);
        self.renderer = Some(renderer);
        Ok(())
    }

    fn on_event(&mut self, event: &Event, _context: &mut EngineContext) {
        if let Event::Window(WindowEvent::Resized { width, height }) = event
            && let Some(renderer) = &mut self.renderer
        {
            renderer.resize(*width, *height);
        }
    }

    fn render(&mut self, context: &mut EngineContext) {
        if let Some(renderer) = &mut self.renderer
            && let Err(render_error) = renderer.render_frame()
        {
            error!("frame rendering failed: {render_error}");
            context.request_exit();
        }
    }

    fn shutdown(&mut self, _context: &mut EngineContext) {
        self.renderer = None;
        info!("renderer released");
    }
}
