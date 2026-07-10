use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("workspace root introuvable")
}

fn collect_files(dir: &Path, file_name_suffix: &str, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).expect("répertoire illisible");
    for entry in entries {
        let path = entry.expect("entrée illisible").path();
        if path.is_dir() {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string());
            if matches!(name.as_deref(), Some("target") | Some(".git")) {
                continue;
            }
            collect_files(&path, file_name_suffix, out);
        } else if path.to_string_lossy().ends_with(file_name_suffix) {
            out.push(path);
        }
    }
}

#[test]
fn the_renderer_and_the_asset_pipeline_never_import_the_ecs() {
    let root = workspace_root();
    let guarded = [
        root.join("crates/engine/chaos_renderer"),
        root.join("crates/engine/chaos_assets"),
    ];

    let mut sources = Vec::new();
    for zone in &guarded {
        collect_files(zone, ".rs", &mut sources);
    }

    let leaks: Vec<PathBuf> = sources
        .into_iter()
        .filter(|file| {
            let source = fs::read_to_string(file).expect("source illisible");
            source.contains("chaos_ecs")
        })
        .collect();

    assert!(
        leaks.is_empty(),
        "chaos_ecs fuit dans le renderer ou l'Asset Pipeline : {leaks:?}"
    );
}

#[test]
fn the_ecs_dependency_stays_out_of_renderer_and_assets_manifests() {
    let root = workspace_root();
    let manifests = [
        root.join("crates/engine/chaos_renderer/Cargo.toml"),
        root.join("crates/engine/chaos_assets/Cargo.toml"),
    ];

    let offenders: Vec<&PathBuf> = manifests
        .iter()
        .filter(|manifest| {
            let content = fs::read_to_string(manifest).expect("manifeste illisible");
            content
                .lines()
                .any(|line| line.trim_start().starts_with("chaos_ecs"))
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "chaos_ecs déclaré dans un manifeste interdit : {offenders:?}"
    );
}

#[test]
fn the_ecs_only_knows_the_core() {
    let root = workspace_root();
    let manifest = root.join("crates/engine/chaos_ecs/Cargo.toml");
    let content = fs::read_to_string(&manifest).expect("manifeste illisible");

    let foreign: Vec<&str> = content
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("chaos_") && !line.starts_with("chaos_core"))
        .collect();

    assert!(
        foreign.is_empty(),
        "chaos_ecs dépend d'autres crates moteur que chaos_core : {foreign:?}"
    );
}
