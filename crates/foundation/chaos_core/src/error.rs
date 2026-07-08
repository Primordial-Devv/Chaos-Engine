use std::error::Error;
use std::fmt;

/// Erreur commune à l'ensemble des crates du moteur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChaosError {
    Window(String),
    Engine(String),
}

pub type ChaosResult<T> = Result<T, ChaosError>;

impl fmt::Display for ChaosError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Window(message) => write!(f, "window error: {message}"),
            Self::Engine(message) => write!(f, "engine error: {message}"),
        }
    }
}

impl Error for ChaosError {}
