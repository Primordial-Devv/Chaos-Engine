use std::sync::Arc;

use chaos_core::{ChaosError, ChaosResult, Event};
use log::{debug, error};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent as WinitWindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::config::WindowConfig;
use crate::handle::WindowHandle;
use crate::translate::translate_window_event;

/// Contrat par lequel un hôte (le moteur) est piloté par la boucle d'événements.
///
/// La boucle appartient à l'OS — exigence macOS — : l'hôte ne possède pas de
/// `loop`, il reçoit la vie à travers ces hooks.
pub trait WindowEventHandler {
    fn on_window_ready(&mut self, window: WindowHandle);
    fn on_event(&mut self, event: Event);
    fn on_update(&mut self);
    fn on_redraw(&mut self);
    fn exit_requested(&self) -> bool;
    fn on_shutdown(&mut self);
}

/// Ouvre la fenêtre décrite par `config` et pilote `handler` jusqu'à la sortie.
/// Bloque le thread appelant ; doit être appelé depuis le main thread.
pub fn run_event_loop(
    config: WindowConfig,
    handler: &mut dyn WindowEventHandler,
) -> ChaosResult<()> {
    let event_loop = EventLoop::new().map_err(|e| ChaosError::Window(e.to_string()))?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = WinitApp {
        config,
        handler,
        window: None,
        error: None,
    };

    let run_result = event_loop
        .run_app(&mut app)
        .map_err(|e| ChaosError::Window(e.to_string()));

    let window_error = app.error.take();
    app.handler.on_shutdown();

    run_result?;
    match window_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct WinitApp<'a> {
    config: WindowConfig,
    handler: &'a mut dyn WindowEventHandler,
    window: Option<WindowHandle>,
    error: Option<ChaosError>,
}

impl ApplicationHandler for WinitApp<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_inner_size(LogicalSize::new(
                f64::from(self.config.width),
                f64::from(self.config.height),
            ))
            .with_resizable(self.config.resizable);
        match event_loop.create_window(attributes) {
            Ok(window) => {
                debug!("native window created");
                let handle = WindowHandle::new(Arc::new(window));
                self.window = Some(handle.clone());
                self.handler.on_window_ready(handle);
            }
            Err(os_error) => {
                error!("native window creation failed: {os_error}");
                self.error = Some(ChaosError::Window(os_error.to_string()));
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WinitWindowEvent,
    ) {
        if let WinitWindowEvent::RedrawRequested = event {
            self.handler.on_redraw();
        } else if let Some(translated) = translate_window_event(&event) {
            self.handler.on_event(translated);
        }
        if self.handler.exit_requested() {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            return;
        }
        self.handler.on_update();
        if self.handler.exit_requested() {
            event_loop.exit();
        }
    }
}
