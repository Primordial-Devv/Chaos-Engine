use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

/// Cible sur laquelle le renderer peut créer une surface de présentation.
///
/// Toute fenêtre exposant les handles natifs standard (raw-window-handle)
/// convient : le renderer n'a jamais besoin de connaître la crate fenêtre.
pub trait SurfaceTarget: HasWindowHandle + HasDisplayHandle + Send + Sync {}

impl<T: HasWindowHandle + HasDisplayHandle + Send + Sync> SurfaceTarget for T {}
