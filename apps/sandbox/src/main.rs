use std::process::ExitCode;

use chaos_engine::{
    AppConfig, Color, Engine, EngineConfig, RenderConfig, RuntimeConfig, WindowConfig,
};

mod geometry_demo;

use geometry_demo::GeometryDemo;

fn main() -> ExitCode {
    let config = EngineConfig {
        app: AppConfig {
            name: String::from("Chaos Sandbox"),
        },
        window: WindowConfig {
            title: String::from("Chaos Sandbox"),
            ..WindowConfig::default()
        },
        render: RenderConfig {
            clear_color: Color::rgb(0.10, 0.03, 0.18),
            ..RenderConfig::default()
        },
        runtime: RuntimeConfig {
            headless: std::env::var("CHAOS_HEADLESS").is_ok_and(|value| value == "1"),
            frame_limit: std::env::var("CHAOS_FRAME_LIMIT")
                .ok()
                .and_then(|value| value.parse().ok()),
            pause_on_focus_loss: true,
            ..RuntimeConfig::default()
        },
        ..EngineConfig::default()
    };

    let mut logger = env_logger::Env::default();
    if let Some(filter) = &config.logs.filter {
        logger = logger.default_filter_or(filter.clone());
    }
    env_logger::Builder::from_env(logger).init();

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
