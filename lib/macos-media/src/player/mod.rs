pub mod state;
pub mod listener;

/// Re-exportaciones del estado del reproductor
pub use state::{State, Album, Timeline};

/// Re-exportación del listener
pub use listener::PlayerListener;

/// Enum para diferentes reproductores de medios compatibles
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum MediaPlayer {
    Music,
    Spotify,
    Unknown,
}

impl Default for MediaPlayer {
    fn default() -> Self {
        Self::Unknown
    }
} 