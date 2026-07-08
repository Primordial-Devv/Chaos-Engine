use chaos_core::{ElementState, Event, InputEvent, KeyCode, MouseButton, WindowEvent};
use winit::event::{
    ElementState as WinitElementState, MouseButton as WinitMouseButton, MouseScrollDelta,
    WindowEvent as WinitWindowEvent,
};
use winit::keyboard::{KeyCode as WinitKeyCode, PhysicalKey};

pub(crate) fn translate_window_event(event: &WinitWindowEvent) -> Option<Event> {
    match event {
        WinitWindowEvent::CloseRequested => Some(Event::Window(WindowEvent::CloseRequested)),
        WinitWindowEvent::Resized(size) => Some(Event::Window(WindowEvent::Resized {
            width: size.width,
            height: size.height,
        })),
        WinitWindowEvent::Moved(position) => Some(Event::Window(WindowEvent::Moved {
            x: position.x,
            y: position.y,
        })),
        WinitWindowEvent::Focused(focused) => Some(Event::Window(WindowEvent::Focused(*focused))),
        WinitWindowEvent::ScaleFactorChanged { scale_factor, .. } => {
            Some(Event::Window(WindowEvent::ScaleFactorChanged {
                scale_factor: *scale_factor,
            }))
        }
        WinitWindowEvent::KeyboardInput { event, .. } => Some(Event::Input(InputEvent::Keyboard {
            key: translate_key(event.physical_key),
            state: translate_state(event.state),
            repeat: event.repeat,
        })),
        WinitWindowEvent::MouseInput { state, button, .. } => {
            Some(Event::Input(InputEvent::MouseButton {
                button: translate_button(*button),
                state: translate_state(*state),
            }))
        }
        WinitWindowEvent::CursorMoved { position, .. } => {
            Some(Event::Input(InputEvent::CursorMoved {
                x: position.x,
                y: position.y,
            }))
        }
        WinitWindowEvent::MouseWheel { delta, .. } => {
            let (delta_x, delta_y) = match delta {
                MouseScrollDelta::LineDelta(x, y) => (f64::from(*x), f64::from(*y)),
                MouseScrollDelta::PixelDelta(position) => (position.x, position.y),
            };
            Some(Event::Input(InputEvent::MouseWheel { delta_x, delta_y }))
        }
        WinitWindowEvent::CursorEntered { .. } => Some(Event::Input(InputEvent::CursorEntered)),
        WinitWindowEvent::CursorLeft { .. } => Some(Event::Input(InputEvent::CursorLeft)),
        _ => None,
    }
}

fn translate_state(state: WinitElementState) -> ElementState {
    match state {
        WinitElementState::Pressed => ElementState::Pressed,
        WinitElementState::Released => ElementState::Released,
    }
}

fn translate_button(button: WinitMouseButton) -> MouseButton {
    match button {
        WinitMouseButton::Left => MouseButton::Left,
        WinitMouseButton::Right => MouseButton::Right,
        WinitMouseButton::Middle => MouseButton::Middle,
        WinitMouseButton::Back => MouseButton::Back,
        WinitMouseButton::Forward => MouseButton::Forward,
        WinitMouseButton::Other(id) => MouseButton::Other(id),
    }
}

fn translate_key(key: PhysicalKey) -> KeyCode {
    let PhysicalKey::Code(code) = key else {
        return KeyCode::Unknown;
    };
    match code {
        WinitKeyCode::KeyA => KeyCode::A,
        WinitKeyCode::KeyB => KeyCode::B,
        WinitKeyCode::KeyC => KeyCode::C,
        WinitKeyCode::KeyD => KeyCode::D,
        WinitKeyCode::KeyE => KeyCode::E,
        WinitKeyCode::KeyF => KeyCode::F,
        WinitKeyCode::KeyG => KeyCode::G,
        WinitKeyCode::KeyH => KeyCode::H,
        WinitKeyCode::KeyI => KeyCode::I,
        WinitKeyCode::KeyJ => KeyCode::J,
        WinitKeyCode::KeyK => KeyCode::K,
        WinitKeyCode::KeyL => KeyCode::L,
        WinitKeyCode::KeyM => KeyCode::M,
        WinitKeyCode::KeyN => KeyCode::N,
        WinitKeyCode::KeyO => KeyCode::O,
        WinitKeyCode::KeyP => KeyCode::P,
        WinitKeyCode::KeyQ => KeyCode::Q,
        WinitKeyCode::KeyR => KeyCode::R,
        WinitKeyCode::KeyS => KeyCode::S,
        WinitKeyCode::KeyT => KeyCode::T,
        WinitKeyCode::KeyU => KeyCode::U,
        WinitKeyCode::KeyV => KeyCode::V,
        WinitKeyCode::KeyW => KeyCode::W,
        WinitKeyCode::KeyX => KeyCode::X,
        WinitKeyCode::KeyY => KeyCode::Y,
        WinitKeyCode::KeyZ => KeyCode::Z,
        WinitKeyCode::Digit0 => KeyCode::Digit0,
        WinitKeyCode::Digit1 => KeyCode::Digit1,
        WinitKeyCode::Digit2 => KeyCode::Digit2,
        WinitKeyCode::Digit3 => KeyCode::Digit3,
        WinitKeyCode::Digit4 => KeyCode::Digit4,
        WinitKeyCode::Digit5 => KeyCode::Digit5,
        WinitKeyCode::Digit6 => KeyCode::Digit6,
        WinitKeyCode::Digit7 => KeyCode::Digit7,
        WinitKeyCode::Digit8 => KeyCode::Digit8,
        WinitKeyCode::Digit9 => KeyCode::Digit9,
        WinitKeyCode::F1 => KeyCode::F1,
        WinitKeyCode::F2 => KeyCode::F2,
        WinitKeyCode::F3 => KeyCode::F3,
        WinitKeyCode::F4 => KeyCode::F4,
        WinitKeyCode::F5 => KeyCode::F5,
        WinitKeyCode::F6 => KeyCode::F6,
        WinitKeyCode::F7 => KeyCode::F7,
        WinitKeyCode::F8 => KeyCode::F8,
        WinitKeyCode::F9 => KeyCode::F9,
        WinitKeyCode::F10 => KeyCode::F10,
        WinitKeyCode::F11 => KeyCode::F11,
        WinitKeyCode::F12 => KeyCode::F12,
        WinitKeyCode::Escape => KeyCode::Escape,
        WinitKeyCode::Tab => KeyCode::Tab,
        WinitKeyCode::Space => KeyCode::Space,
        WinitKeyCode::Enter => KeyCode::Enter,
        WinitKeyCode::Backspace => KeyCode::Backspace,
        WinitKeyCode::Delete => KeyCode::Delete,
        WinitKeyCode::Insert => KeyCode::Insert,
        WinitKeyCode::Home => KeyCode::Home,
        WinitKeyCode::End => KeyCode::End,
        WinitKeyCode::PageUp => KeyCode::PageUp,
        WinitKeyCode::PageDown => KeyCode::PageDown,
        WinitKeyCode::ArrowUp => KeyCode::ArrowUp,
        WinitKeyCode::ArrowDown => KeyCode::ArrowDown,
        WinitKeyCode::ArrowLeft => KeyCode::ArrowLeft,
        WinitKeyCode::ArrowRight => KeyCode::ArrowRight,
        WinitKeyCode::ShiftLeft => KeyCode::ShiftLeft,
        WinitKeyCode::ShiftRight => KeyCode::ShiftRight,
        WinitKeyCode::ControlLeft => KeyCode::ControlLeft,
        WinitKeyCode::ControlRight => KeyCode::ControlRight,
        WinitKeyCode::AltLeft => KeyCode::AltLeft,
        WinitKeyCode::AltRight => KeyCode::AltRight,
        WinitKeyCode::SuperLeft => KeyCode::SuperLeft,
        WinitKeyCode::SuperRight => KeyCode::SuperRight,
        WinitKeyCode::CapsLock => KeyCode::CapsLock,
        WinitKeyCode::NumLock => KeyCode::NumLock,
        WinitKeyCode::PrintScreen => KeyCode::PrintScreen,
        WinitKeyCode::ScrollLock => KeyCode::ScrollLock,
        WinitKeyCode::Pause => KeyCode::Pause,
        WinitKeyCode::ContextMenu => KeyCode::ContextMenu,
        WinitKeyCode::Minus => KeyCode::Minus,
        WinitKeyCode::Equal => KeyCode::Equal,
        WinitKeyCode::BracketLeft => KeyCode::BracketLeft,
        WinitKeyCode::BracketRight => KeyCode::BracketRight,
        WinitKeyCode::Backslash => KeyCode::Backslash,
        WinitKeyCode::Semicolon => KeyCode::Semicolon,
        WinitKeyCode::Quote => KeyCode::Quote,
        WinitKeyCode::Comma => KeyCode::Comma,
        WinitKeyCode::Period => KeyCode::Period,
        WinitKeyCode::Slash => KeyCode::Slash,
        WinitKeyCode::Backquote => KeyCode::Backquote,
        WinitKeyCode::Numpad0 => KeyCode::Numpad0,
        WinitKeyCode::Numpad1 => KeyCode::Numpad1,
        WinitKeyCode::Numpad2 => KeyCode::Numpad2,
        WinitKeyCode::Numpad3 => KeyCode::Numpad3,
        WinitKeyCode::Numpad4 => KeyCode::Numpad4,
        WinitKeyCode::Numpad5 => KeyCode::Numpad5,
        WinitKeyCode::Numpad6 => KeyCode::Numpad6,
        WinitKeyCode::Numpad7 => KeyCode::Numpad7,
        WinitKeyCode::Numpad8 => KeyCode::Numpad8,
        WinitKeyCode::Numpad9 => KeyCode::Numpad9,
        WinitKeyCode::NumpadAdd => KeyCode::NumpadAdd,
        WinitKeyCode::NumpadSubtract => KeyCode::NumpadSubtract,
        WinitKeyCode::NumpadMultiply => KeyCode::NumpadMultiply,
        WinitKeyCode::NumpadDivide => KeyCode::NumpadDivide,
        WinitKeyCode::NumpadDecimal => KeyCode::NumpadDecimal,
        WinitKeyCode::NumpadEnter => KeyCode::NumpadEnter,
        _ => KeyCode::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::NativeKeyCode;

    #[test]
    fn states_are_translated() {
        assert_eq!(
            translate_state(WinitElementState::Pressed),
            ElementState::Pressed
        );
        assert_eq!(
            translate_state(WinitElementState::Released),
            ElementState::Released
        );
    }

    #[test]
    fn buttons_are_translated() {
        assert_eq!(translate_button(WinitMouseButton::Left), MouseButton::Left);
        assert_eq!(
            translate_button(WinitMouseButton::Other(7)),
            MouseButton::Other(7)
        );
    }

    #[test]
    fn known_keys_are_translated() {
        assert_eq!(
            translate_key(PhysicalKey::Code(WinitKeyCode::KeyA)),
            KeyCode::A
        );
        assert_eq!(
            translate_key(PhysicalKey::Code(WinitKeyCode::Escape)),
            KeyCode::Escape
        );
        assert_eq!(
            translate_key(PhysicalKey::Code(WinitKeyCode::NumpadEnter)),
            KeyCode::NumpadEnter
        );
    }

    #[test]
    fn unidentified_keys_fall_back_to_unknown() {
        assert_eq!(
            translate_key(PhysicalKey::Unidentified(NativeKeyCode::Unidentified)),
            KeyCode::Unknown
        );
    }
}
