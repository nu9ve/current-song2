#![cfg(target_os = "macos")]

/// Módulo para acceder a información de reproductores de medios en macOS 
/// a través de AppleScript
pub mod player;

// Re-exportamos las estructuras más importantes para facilitar el uso
pub use player::{PlayerListener, State, Album, Timeline, MediaPlayer};

/// Módulo para obtener información de reproductores de medios en macOS
#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
} 