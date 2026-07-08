use std::sync::Arc;

use winit::window::Window;

/// Poignée opaque sur la fenêtre native, seul accès exposé hors de la crate.
/// Portera les raw handles nécessaires au renderer dans une phase ultérieure.
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
