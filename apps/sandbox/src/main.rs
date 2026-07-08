use std::process::ExitCode;

use chaos_engine::{Engine, EngineConfig, WindowConfig};

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
        ..EngineConfig::default()
    };

    match Engine::new(config).run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("chaos engine terminated with an error: {error}");
            ExitCode::FAILURE
        }
    }
}
