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

/// « Les applications ne voient QUE la façade » : `apps/` consomme
/// `chaos_engine` (ou une crate PLATEFORME — le modèle 4 couches), jamais
/// une crate interne du moteur. Verrouillé sur les manifestes ET les
/// imports (le code, jamais les commentaires — le patron des verrous
/// d'isolation).
#[test]
fn applications_see_only_the_facade() {
    let internal = [
        "chaos_core",
        "chaos_ecs",
        "chaos_scene",
        "chaos_renderer",
        "chaos_window",
        "chaos_assets",
        "chaos_network",
        "chaos_physics",
        "chaos_audio",
    ];
    let root = workspace_root();

    let mut manifests = Vec::new();
    collect_files(&root.join("apps"), "Cargo.toml", &mut manifests);
    let manifest_offenders: Vec<(PathBuf, &str)> = manifests
        .into_iter()
        .filter_map(|manifest| {
            let contents = fs::read_to_string(&manifest).expect("manifeste illisible");
            internal
                .iter()
                .find(|name| {
                    contents
                        .lines()
                        .any(|line| line.trim_start().starts_with(&format!("{name}.workspace")))
                })
                .map(|name| (manifest, *name))
        })
        .collect();
    assert!(
        manifest_offenders.is_empty(),
        "une application dépend d'une crate interne : {manifest_offenders:?}"
    );

    let mut sources = Vec::new();
    collect_files(&root.join("apps"), ".rs", &mut sources);
    let import_offenders: Vec<(PathBuf, String)> = sources
        .into_iter()
        .filter_map(|file| {
            let source = fs::read_to_string(&file).expect("source illisible");
            internal
                .iter()
                .find(|name| {
                    source.contains(&format!("use {name}")) || source.contains(&format!("{name}::"))
                })
                .map(|name| (file, String::from(*name)))
        })
        .collect();
    assert!(
        import_offenders.is_empty(),
        "une application importe une crate interne : {import_offenders:?}"
    );
}

/// « Pas d'accès globaux cachés, pas de singletons incontrôlés » : les
/// services passent par `EngineContext`, jamais par un état global. Ce
/// verrou protège l'avenir, pas seulement le présent.
#[test]
fn the_engine_has_no_hidden_globals() {
    let root = workspace_root();
    // Les motifs sont assemblés pour que CE fichier ne se piège pas
    // lui-même (le patron du verrou wgpu).
    let forbidden = [
        ["static", " mut "].concat(),
        ["lazy", "_static"].concat(),
        ["once", "_cell"].concat(),
        ["Once", "Lock"].concat(),
        ["Once", "Cell"].concat(),
        ["thread", "_local!"].concat(),
    ];

    let mut sources = Vec::new();
    collect_files(&root.join("crates"), ".rs", &mut sources);
    collect_files(&root.join("apps"), ".rs", &mut sources);

    let offenders: Vec<(PathBuf, String)> = sources
        .into_iter()
        .filter(|file| !file.ends_with("tests/boundaries.rs"))
        .filter_map(|file| {
            let source = fs::read_to_string(&file).expect("source illisible");
            forbidden
                .iter()
                .find(|pattern| source.contains(pattern.as_str()))
                .map(|pattern| (file, pattern.clone()))
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "état global caché détecté : {offenders:?}"
    );
}
