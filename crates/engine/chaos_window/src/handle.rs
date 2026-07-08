use std::sync::Arc;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle as RawWindowHandle,
};
use winit::window::Window;

/// Poignée opaque sur la fenêtre native, seul accès exposé hors de la crate.
/// Expose les handles natifs standard (raw-window-handle) pour que le renderer
/// puisse créer sa surface sans dépendre de cette crate.
#[derive(Debug, Clone)]
pub struct WindowHandle {
    window: Arc<Window>,
}

impl WindowHandle {
    pub(crate) fn new(window: Arc<Window>) -> Self {
        Self { window }
    }

    pub fn inner_size(&self) -> (u32, u32) {
        let size = self.window.inner_size();
        (size.width, size.height)
    }

    pub fn scale_factor(&self) -> f64 {
        self.window.scale_factor()
    }

    pub fn set_title(&self, title: &str) {
        self.window.set_title(title);
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }
}

impl HasWindowHandle for WindowHandle {
    fn window_handle(&self) -> Result<RawWindowHandle<'_>, HandleError> {
        self.window.window_handle()
    }
}

impl HasDisplayHandle for WindowHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.window.display_handle()
    }
}
