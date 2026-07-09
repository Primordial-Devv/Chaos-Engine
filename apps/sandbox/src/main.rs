use std::process::ExitCode;

use chaos_engine::{Color, Engine, EngineConfig, WindowConfig};

mod geometry_demo;

use geometry_demo::GeometryDemo;

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let config = EngineConfig {
        app_name: String::from("Chaos Sandbox"),
        window: WindowConfig {
            title: String::from("Chaos Sandbox"),
            ..WindowConfig::default()
        },
        frame_limit: std::env::var("CHAOS_FRAME_LIMIT")
            .ok()
            .and_then(|value| value.parse().ok()),
        clear_color: Color::rgb(0.10, 0.03, 0.18),
        ..EngineConfig::default()
    };

    let mut engine = Engine::new(config);
    engine.add_subsystem(Box::new(GeometryDemo::default()));

    match engine.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("chaos engine terminated with an error: {error}");
            ExitCode::FAILURE
        }
    }
}
