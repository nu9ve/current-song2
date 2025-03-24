use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Estado del reproductor de medios
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub title: String,
    pub artist: String,
    pub album: Option<Album>,
    pub track_id: Option<String>,
    pub track_number: Option<u32>,
    pub timeline: Option<Timeline>,
    pub player_name: String,
    pub timestamp: DateTime<Utc>,
    pub artwork_data: Option<Vec<u8>>,
}

/// Información del álbum
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Album {
    pub title: String,
    pub track_count: u32,
}

/// Información de la línea de tiempo de reproducción
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub progress: Duration,
    pub duration: Duration,
}

impl Default for State {
    fn default() -> Self {
        Self {
            title: String::new(),
            artist: String::new(),
            album: None,
            track_id: None,
            track_number: None,
            timeline: None,
            player_name: String::new(),
            timestamp: Utc::now(),
            artwork_data: None,
        }
    }
}

impl State {
    /// Crea un estado de pausa/sin reproducción
    pub fn paused() -> Self {
        Self::default()
    }
} 