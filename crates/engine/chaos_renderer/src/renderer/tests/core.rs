//! Le socle du renderer : propagation du backend (outcome, description),
//! pipelines explicites, résolution des shaders et contrat `Send`.

use super::*;

#[test]
fn backend_outcome_is_propagated() {
    let skipped = FrameOutcome::Skipped(FrameSkipReason::SurfaceUnavailable);
    let (mut renderer, _journal) = mock_renderer_with(skipped);
    assert_eq!(renderer.render_frame().unwrap(), skipped);
}

#[test]
fn description_delegates_to_backend() {
    let (renderer, _journal) = mock_renderer();
    assert_eq!(renderer.description(), "mock backend");
}

#[test]
fn create_pipeline_returns_increasing_handles() {
    let (mut renderer, _journal) = mock_renderer();
    let first = renderer.create_pipeline(&inline_descriptor("a")).unwrap();
    let second = renderer.create_pipeline(&inline_descriptor("b")).unwrap();
    assert_ne!(first, second);
}

#[test]
fn inline_shader_reaches_the_backend() {
    let (mut renderer, journal) = mock_renderer();
    renderer.create_pipeline(&inline_descriptor("a")).unwrap();
    assert_eq!(
        journal.entries(),
        vec!["create_pipeline a code=inline-code"]
    );
}

#[test]
fn named_shader_resolves_through_the_library() {
    let (mut renderer, journal) = mock_renderer();
    renderer.shaders_mut().register(
        "game.custom",
        ShaderSource::Wgsl(String::from("custom-code")),
    );
    let descriptor = PipelineDescriptor::new("t", "game.custom");
    renderer.create_pipeline(&descriptor).unwrap();
    assert_eq!(
        journal.entries(),
        vec!["create_pipeline t code=custom-code"]
    );
}

#[test]
fn unknown_named_shader_is_a_comprehensible_error() {
    let (mut renderer, journal) = mock_renderer();
    let descriptor = PipelineDescriptor::new("t", "missing.shader");
    let error = renderer.create_pipeline(&descriptor).unwrap_err();
    assert!(error.to_string().contains("missing.shader"));
    assert!(journal.entries().is_empty());
}

#[test]
fn builtin_vertex_color_is_available() {
    let (renderer, _journal) = mock_renderer();
    assert!(renderer.shaders().contains(builtin::VERTEX_COLOR));
}

#[test]
fn the_renderer_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Renderer>();
}
