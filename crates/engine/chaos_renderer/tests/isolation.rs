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
fn wgpu_never_leaks_outside_the_backend() {
    let root = workspace_root();
    let allowed_zone = root.join("crates/engine/chaos_renderer/src/backend");
    let use_pattern = ["use ", "wgpu"].concat();
    let path_pattern = ["wgpu", "::"].concat();

    let mut sources = Vec::new();
    collect_files(&root.join("crates"), ".rs", &mut sources);
    collect_files(&root.join("apps"), ".rs", &mut sources);

    let leaks: Vec<PathBuf> = sources
        .into_iter()
        .filter(|file| !file.starts_with(&allowed_zone))
        .filter(|file| {
            let source = fs::read_to_string(file).expect("source illisible");
            source.contains(&use_pattern) || source.contains(&path_pattern)
        })
        .collect();

    assert!(
        leaks.is_empty(),
        "wgpu fuit hors de chaos_renderer/src/backend/ : {leaks:?}"
    );
}

#[test]
fn wgpu_dependency_lives_in_a_single_manifest() {
    let root = workspace_root();

    let mut manifests = Vec::new();
    collect_files(&root.join("crates"), "Cargo.toml", &mut manifests);
    collect_files(&root.join("apps"), "Cargo.toml", &mut manifests);

    let offenders: Vec<PathBuf> = manifests
        .into_iter()
        .filter(|manifest| !manifest.ends_with("chaos_renderer/Cargo.toml"))
        .filter(|manifest| {
            let content = fs::read_to_string(manifest).expect("manifeste illisible");
            content
                .lines()
                .any(|line| line.trim_start().starts_with("wgpu"))
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "wgpu déclaré hors du manifeste de chaos_renderer : {offenders:?}"
    );
}
