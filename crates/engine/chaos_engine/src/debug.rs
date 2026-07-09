use std::collections::HashSet;
use std::f32::consts::PI;

use chaos_core::math::{Quat, Vec3, world};
use chaos_core::{Camera, ElementState, Event, InputEvent, KeyCode, MouseButton, WindowEvent};

const MAX_PITCH: f32 = 89.0 * PI / 180.0;
const MIN_SPEED: f32 = 0.1;
const MAX_SPEED: f32 = 100.0;

/// Contrôleur de caméra de DEBUG — outil de développement, pas un système
/// gameplay : isolé, remplaçable, à brancher par l'app qui possède sa caméra.
///
/// Navigation : touches physiques WASD (= ZQSD sur AZERTY), Space monte,
/// Shift gauche descend, vol libre. Regard : bouton droit maintenu + souris.
/// Molette : multiplicateur de vitesse. Focus perdu : entrées purgées.
pub struct DebugCameraController {
    pub move_speed: f32,
    pub look_sensitivity: f32,
    held: HashSet<KeyCode>,
    looking: bool,
    last_cursor: Option<(f64, f64)>,
    yaw: f32,
    pitch: f32,
    pending_yaw: f32,
    pending_pitch: f32,
}

impl Default for DebugCameraController {
    fn default() -> Self {
        Self {
            move_speed: 3.0,
            look_sensitivity: 0.003,
            held: HashSet::new(),
            looking: false,
            last_cursor: None,
            yaw: 0.0,
            pitch: 0.0,
            pending_yaw: 0.0,
            pending_pitch: 0.0,
        }
    }
}

impl DebugCameraController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle_event(&mut self, event: &Event) {
        match event {
            Event::Input(InputEvent::Keyboard { key, state, .. }) => match state {
                ElementState::Pressed => {
                    self.held.insert(*key);
                }
                ElementState::Released => {
                    self.held.remove(key);
                }
            },
            Event::Input(InputEvent::MouseButton {
                button: MouseButton::Right,
                state,
            }) => {
                self.looking = *state == ElementState::Pressed;
                self.last_cursor = None;
            }
            Event::Input(InputEvent::CursorMoved { x, y }) => {
                if self.looking {
                    if let Some((last_x, last_y)) = self.last_cursor {
                        self.pending_yaw -= (x - last_x) as f32 * self.look_sensitivity;
                        self.pending_pitch -= (y - last_y) as f32 * self.look_sensitivity;
                    }
                    self.last_cursor = Some((*x, *y));
                }
            }
            Event::Input(InputEvent::MouseWheel { delta_y, .. }) => {
                let factor = (1.0 + 0.1 * *delta_y as f32).clamp(0.5, 2.0);
                self.move_speed = (self.move_speed * factor).clamp(MIN_SPEED, MAX_SPEED);
            }
            Event::Input(InputEvent::CursorLeft) => {
                self.last_cursor = None;
            }
            Event::Window(WindowEvent::Focused(false)) => {
                self.held.clear();
                self.looking = false;
                self.last_cursor = None;
            }
            _ => {}
        }
    }

    pub fn update(&mut self, camera: &mut Camera, delta_seconds: f32) {
        self.yaw += self.pending_yaw;
        self.pitch = (self.pitch + self.pending_pitch).clamp(-MAX_PITCH, MAX_PITCH);
        self.pending_yaw = 0.0;
        self.pending_pitch = 0.0;
        camera.transform.rotation =
            Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch);

        let mut direction = Vec3::ZERO;
        if self.held.contains(&KeyCode::W) {
            direction += camera.transform.forward();
        }
        if self.held.contains(&KeyCode::S) {
            direction -= camera.transform.forward();
        }
        if self.held.contains(&KeyCode::D) {
            direction += camera.transform.right();
        }
        if self.held.contains(&KeyCode::A) {
            direction -= camera.transform.right();
        }
        if self.held.contains(&KeyCode::Space) {
            direction += world::UP;
        }
        if self.held.contains(&KeyCode::ShiftLeft) {
            direction -= world::UP;
        }
        if direction != Vec3::ZERO {
            camera.transform.translation += direction.normalize() * self.move_speed * delta_seconds;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pressed(key: KeyCode) -> Event {
        Event::Input(InputEvent::Keyboard {
            key,
            state: ElementState::Pressed,
            repeat: false,
        })
    }

    fn right_button(state: ElementState) -> Event {
        Event::Input(InputEvent::MouseButton {
            button: MouseButton::Right,
            state,
        })
    }

    fn cursor(x: f64, y: f64) -> Event {
        Event::Input(InputEvent::CursorMoved { x, y })
    }

    fn nearly(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn forward_key_moves_along_camera_forward() {
        let mut controller = DebugCameraController::new();
        let mut camera = Camera::default();
        controller.handle_event(&pressed(KeyCode::W));
        controller.update(&mut camera, 2.0);
        assert!(nearly(
            camera.transform.translation,
            Vec3::new(0.0, 0.0, -6.0)
        ));
    }

    #[test]
    fn focus_loss_releases_held_keys() {
        let mut controller = DebugCameraController::new();
        let mut camera = Camera::default();
        controller.handle_event(&pressed(KeyCode::W));
        controller.handle_event(&Event::Window(WindowEvent::Focused(false)));
        controller.update(&mut camera, 1.0);
        assert!(nearly(camera.transform.translation, Vec3::ZERO));
    }

    #[test]
    fn right_drag_rotates_the_camera() {
        let mut controller = DebugCameraController::new();
        let mut camera = Camera::default();
        controller.handle_event(&right_button(ElementState::Pressed));
        controller.handle_event(&cursor(100.0, 100.0));
        controller.handle_event(&cursor(200.0, 100.0));
        controller.update(&mut camera, 0.0);
        assert!(camera.transform.forward().x > 0.05);
    }

    #[test]
    fn cursor_motion_without_drag_does_not_rotate() {
        let mut controller = DebugCameraController::new();
        let mut camera = Camera::default();
        controller.handle_event(&cursor(100.0, 100.0));
        controller.handle_event(&cursor(500.0, 300.0));
        controller.update(&mut camera, 0.0);
        assert!(nearly(camera.transform.forward(), Vec3::NEG_Z));
    }

    #[test]
    fn first_drag_motion_causes_no_jump() {
        let mut controller = DebugCameraController::new();
        let mut camera = Camera::default();
        controller.handle_event(&right_button(ElementState::Pressed));
        controller.handle_event(&cursor(4000.0, -2500.0));
        controller.update(&mut camera, 0.0);
        assert!(nearly(camera.transform.forward(), Vec3::NEG_Z));
    }

    #[test]
    fn pitch_is_clamped() {
        let mut controller = DebugCameraController::new();
        let mut camera = Camera::default();
        controller.handle_event(&right_button(ElementState::Pressed));
        controller.handle_event(&cursor(0.0, 0.0));
        controller.handle_event(&cursor(0.0, -100_000.0));
        controller.update(&mut camera, 0.0);
        let up_component = camera.transform.forward().y;
        assert!(up_component > 0.98 && up_component < 1.0);
    }

    #[test]
    fn wheel_scales_speed_within_bounds() {
        let mut controller = DebugCameraController::new();
        for _ in 0..200 {
            controller.handle_event(&Event::Input(InputEvent::MouseWheel {
                delta_x: 0.0,
                delta_y: 5.0,
            }));
        }
        assert!((controller.move_speed - MAX_SPEED).abs() < 1e-3);
        for _ in 0..200 {
            controller.handle_event(&Event::Input(InputEvent::MouseWheel {
                delta_x: 0.0,
                delta_y: -5.0,
            }));
        }
        assert!((controller.move_speed - MIN_SPEED).abs() < 1e-3);
    }
}
