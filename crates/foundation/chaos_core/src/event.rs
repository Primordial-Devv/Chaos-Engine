use crate::input::{ElementState, KeyCode, MouseButton};

/// Événement du moteur, déjà traduit dans le vocabulaire maison :
/// aucun type du backend fenêtre ne fuit au-delà de chaos_window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    Window(WindowEvent),
    Input(InputEvent),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowEvent {
    CloseRequested,
    Resized { width: u32, height: u32 },
    Moved { x: i32, y: i32 },
    Focused(bool),
    ScaleFactorChanged { scale_factor: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputEvent {
    Keyboard {
        key: KeyCode,
        state: ElementState,
        repeat: bool,
    },
    MouseButton {
        button: MouseButton,
        state: ElementState,
    },
    CursorMoved {
        x: f64,
        y: f64,
    },
    MouseWheel {
        delta_x: f64,
        delta_y: f64,
    },
    CursorEntered,
    CursorLeft,
}
