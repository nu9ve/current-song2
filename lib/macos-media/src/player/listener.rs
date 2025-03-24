use crate::player::state::{State, Album, Timeline};
use chrono::Utc;
use serde_json::Value;
use std::time::Duration;
use tokio::{process::Command, sync::mpsc};
use tracing::{debug, error, info, warn};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("AppleScript error: {0}")]
    AppleScript(String),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize)]
struct TrackInfo {
    title: String,
    artist: String,
    album: AlbumInfo,
    #[serde(default)]
    track_number: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct AlbumInfo {
    title: String,
    track_count: u32,
}

#[derive(Debug, Deserialize)]
struct TimelineInfo {
    progress: f64,
    duration: f64,
}

/// Listener para detectar el estado de reproducción de los reproductores multimedia
#[derive(Debug, Clone)]
pub struct PlayerListener {
    /// Intervalo de tiempo entre verificaciones (en milisegundos)
    interval: Duration,
    /// Lista de reproductores a verificar
    players: Vec<String>,
}

impl PlayerListener {
    /// Crea un nuevo listener con los reproductores especificados
    pub fn new(players: Vec<String>, interval_ms: u64) -> Self {
        Self {
            interval: Duration::from_millis(interval_ms),
            players,
        }
    }

    /// Crea un nuevo listener con configuración predeterminada
    pub fn default() -> Self {
        Self {
            interval: Duration::from_millis(1000),
            players: vec!["Spotify".to_string(), "Music".to_string()],
        }
    }
    
    /// Verificar si Spotify está reproduciendo música
    pub fn check_spotify(&self) -> Option<State> {
        // Verificar si Spotify está ejecutándose
        let is_running = match run_applescript("tell application \"System Events\" to (name of processes) contains \"Spotify\"") {
            Ok(output) => output.trim() == "true",
            Err(e) => {
                warn!("Error comprobando si Spotify está ejecutándose: {}", e);
                return None;
            }
        };

        if !is_running {
            debug!("Spotify no está ejecutándose");
            return None;
        }

        // Verificar si está reproduciendo música
        let is_playing = match run_applescript("tell application \"Spotify\" to player state as string") {
            Ok(output) => output.trim() == "playing",
            Err(e) => {
                warn!("Error comprobando el estado de reproducción de Spotify: {}", e);
                return None;
            }
        };

        if !is_playing {
            debug!("Spotify no está reproduciendo");
            return None;
        }

        // Obtener información de la canción con escape de caracteres
        let script = r#"
            tell application "Spotify"
                set currentTrack to current track
                set trackName to name of currentTrack as string
                set artistName to artist of currentTrack as string
                set albumName to album of currentTrack as string
                
                set trackNumber to "null"
                try
                    if track number of currentTrack is not missing value then
                        set trackNumber to track number of currentTrack
                    end if
                end try
                
                set output to "{"
                set output to output & "\"title\": \"" & my escape_quotes(trackName) & "\", "
                set output to output & "\"artist\": \"" & my escape_quotes(artistName) & "\", "
                set output to output & "\"album\": {"
                set output to output & "\"title\": \"" & my escape_quotes(albumName) & "\", "
                set output to output & "\"track_count\": 0"
                set output to output & "}"
                
                if trackNumber is not "null" then
                    set output to output & ", \"track_number\": " & trackNumber
                end if
                
                set output to output & "}"
                return output
            end tell
            
            on escape_quotes(theText)
                set AppleScript's text item delimiters to "\""
                set theTextItems to every text item of theText
                set AppleScript's text item delimiters to "\\\""
                set theText to theTextItems as string
                set AppleScript's text item delimiters to ""
                return theText
            end escape_quotes
        "#;

        let info_str = match run_applescript(script) {
            Ok(output) => output,
            Err(e) => {
                warn!("Error obteniendo información de la canción en Spotify: {}", e);
                return None;
            }
        };

        let track_info: TrackInfo = match serde_json::from_str(&info_str) {
            Ok(info) => info,
            Err(e) => {
                warn!("Error parseando JSON de Spotify: {} - Raw: {}", e, info_str);
                return None;
            }
        };

        // Obtener información de tiempo
        let timeline_script = r#"
            tell application "Spotify"
                set progress_secs to 0
                set duration_secs to 0
                
                try
                    set progress_secs to player position
                    set duration_secs to duration of current track
                end try
                
                set json to "{"
                set json to json & "\"progress\": " & progress_secs & ", "
                set json to json & "\"duration\": " & duration_secs
                set json to json & "}"
                return json
            end tell
        "#;

        let timeline_info: Option<TimelineInfo> = match run_applescript(timeline_script) {
            Ok(output) => match serde_json::from_str(&output) {
                Ok(info) => Some(info),
                Err(e) => {
                    warn!("Error parseando JSON de línea de tiempo de Spotify: {}", e);
                    None
                }
            },
            Err(e) => {
                warn!("Error obteniendo línea de tiempo de Spotify: {}", e);
                None
            }
        };

        // Construir el timeline
        let timeline = if let Some(timeline_info) = timeline_info {
            Some(Timeline {
                progress: Duration::from_secs_f64(timeline_info.progress),
                duration: Duration::from_secs_f64(timeline_info.duration),
            })
        } else {
            None
        };

        // Construir el album
        let album = Some(Album {
            title: track_info.album.title,
            track_count: track_info.album.track_count,
        });

        debug!("Canción detectada en Spotify: {} - {}", track_info.title, track_info.artist);

        Some(State {
            player_name: "Spotify".to_string(),
            title: track_info.title,
            artist: track_info.artist,
            album,
            track_number: track_info.track_number,
            timeline,
            artwork_data: None, // Aquí no cargamos la imagen, se cargará solo cuando cambie la canción
            timestamp: Utc::now(),
            track_id: None,
        })
    }

    /// Verificar reproductor Music (iTunes)
    pub fn check_music(&self) -> Option<State> {
        // Verificar si Music está ejecutándose
        let is_running = match run_applescript("tell application \"System Events\" to (name of processes) contains \"Music\"") {
            Ok(output) => output.trim() == "true",
            Err(e) => {
                warn!("Error comprobando si Music está ejecutándose: {}", e);
                return None;
            }
        };

        if !is_running {
            debug!("Music no está ejecutándose");
            return None;
        }

        // Verificar si está reproduciendo música
        let is_playing = match run_applescript("tell application \"Music\" to player state as string") {
            Ok(output) => output.trim() == "playing",
            Err(e) => {
                warn!("Error comprobando el estado de reproducción de Music: {}", e);
                return None;
            }
        };

        if !is_playing {
            debug!("Music no está reproduciendo");
            return None;
        }

        // Obtener información de la canción con escape de caracteres
        let script = r#"
            tell application "Music"
                set currentTrack to current track
                set trackName to name of currentTrack as string
                set artistName to artist of currentTrack as string
                set albumName to album of currentTrack as string
                
                set trackCount to 0
                try
                    set trackCount to track count of (get album of currentTrack)
                end try
                
                set trackNumber to "null"
                try
                    if track number of currentTrack is not missing value then
                        set trackNumber to track number of currentTrack
                    end if
                end try
                
                set output to "{"
                set output to output & "\"title\": \"" & my escape_quotes(trackName) & "\", "
                set output to output & "\"artist\": \"" & my escape_quotes(artistName) & "\", "
                set output to output & "\"album\": {"
                set output to output & "\"title\": \"" & my escape_quotes(albumName) & "\", "
                set output to output & "\"track_count\": " & trackCount
                set output to output & "}"
                
                if trackNumber is not "null" then
                    set output to output & ", \"track_number\": " & trackNumber
                end if
                
                set output to output & "}"
                return output
            end tell
            
            on escape_quotes(theText)
                set AppleScript's text item delimiters to "\""
                set theTextItems to every text item of theText
                set AppleScript's text item delimiters to "\\\""
                set theText to theTextItems as string
                set AppleScript's text item delimiters to ""
                return theText
            end escape_quotes
        "#;

        let info_str = match run_applescript(script) {
            Ok(output) => output,
            Err(e) => {
                warn!("Error obteniendo información de la canción en Music: {}", e);
                return None;
            }
        };

        let track_info: TrackInfo = match serde_json::from_str(&info_str) {
            Ok(info) => info,
            Err(e) => {
                warn!("Error parseando JSON de Music: {} - Raw: {}", e, info_str);
                return None;
            }
        };

        // Obtener información de tiempo
        let timeline_script = r#"
            tell application "Music"
                set progress_secs to 0
                set duration_secs to 0
                
                try
                    set progress_secs to player position
                    set duration_secs to duration of current track
                end try
                
                set json to "{"
                set json to json & "\"progress\": " & progress_secs & ", "
                set json to json & "\"duration\": " & duration_secs
                set json to json & "}"
                return json
            end tell
        "#;

        let timeline_info: Option<TimelineInfo> = match run_applescript(timeline_script) {
            Ok(output) => match serde_json::from_str(&output) {
                Ok(info) => Some(info),
                Err(e) => {
                    warn!("Error parseando JSON de línea de tiempo de Music: {}", e);
                    None
                }
            },
            Err(e) => {
                warn!("Error obteniendo línea de tiempo de Music: {}", e);
                None
            }
        };

        // Construir el timeline
        let timeline = if let Some(timeline_info) = timeline_info {
            Some(Timeline {
                progress: Duration::from_secs_f64(timeline_info.progress),
                duration: Duration::from_secs_f64(timeline_info.duration),
            })
        } else {
            None
        };

        // Construir el album
        let album = Some(Album {
            title: track_info.album.title,
            track_count: track_info.album.track_count,
        });

        Some(State {
            player_name: "Music".to_string(),
            title: track_info.title,
            artist: track_info.artist,
            album,
            track_number: track_info.track_number,
            timeline,
            artwork_data: None, // Aquí no cargamos la imagen, se cargará solo cuando cambie la canción
            timestamp: Utc::now(),
            track_id: None,
        })
    }

    /// Verifica todos los reproductores de música disponibles
    /// 
    /// Devuelve el estado del primer reproductor que esté reproduciendo música
    pub fn check_all_players(&self) -> Option<State> {
        debug!("Comprobando todos los reproductores de música...");
        
        // Primero intentamos con Spotify
        if let Some(state) = self.check_spotify() {
            return Some(state);
        }
        
        // Luego intentamos con Music (iTunes)
        if let Some(state) = self.check_music() {
            return Some(state);
        }
        
        debug!("No se encontró ningún reproductor activo");
        None
    }

    /// Iniciar un listener para detectar cambios en la reproducción de medios
    pub fn listen(&self, tx: mpsc::Sender<State>) -> tokio::task::JoinHandle<()> {
        let interval = self.interval;
        let listener = self.clone();
        
        tokio::spawn(async move {
            debug!("Iniciando la escucha de reproductores de música cada {}ms", interval.as_millis());
            
            let mut last_state: Option<State> = None;
            let mut log_counter = 0;
            let log_frequency = 45; // Log cada 45 intervalos (aprox. 45 segundos con interval=1000ms)
            
            // Para control de obtención de imagen
            let mut last_track_id: Option<String> = None;
            let mut last_artwork_update = 0;
            
            loop {
                log_counter += 1;
                
                match listener.check_all_players() {
                    Some(mut current_state) => {
                        // Generar un ID único para la canción actual
                        let track_id = format!("{}-{}-{}", 
                            current_state.player_name, 
                            current_state.title, 
                            current_state.artist);
                            
                        // Verificar si es necesario obtener la imagen (nueva canción o primera vez)
                        let need_artwork = match &last_track_id {
                            Some(last_id) => *last_id != track_id || current_state.artwork_data.is_none(),
                            None => true,
                        };
                        
                        if need_artwork {
                            // Si es una canción nueva, obtenemos la portada
                            current_state = match current_state.player_name.as_str() {
                                "Music" => listener.get_music_artwork(current_state),
                                "Spotify" => listener.get_spotify_artwork(current_state),
                                _ => current_state,
                            };
                            
                            // Actualizamos el ID de la última canción
                            last_track_id = Some(track_id);
                            last_artwork_update = 0;
                        } else if let Some(last) = &last_state {
                            // Para canción existente, reutilizamos la portada si está disponible
                            if current_state.artwork_data.is_none() && last.artwork_data.is_some() {
                                current_state.artwork_data = last.artwork_data.clone();
                            }
                        }
                        
                        // Si el estado cambió o es la primera detección
                        if should_update_state(&last_state, &current_state) {
                            // Si hay un cambio real en la canción, forzamos un log inmediato
                            let song_changed = last_state.as_ref()
                                .map(|s| s.title != current_state.title || s.artist != current_state.artist)
                                .unwrap_or(true);
                                
                            if song_changed {
                                let progress_duration = format_duration(&current_state.timeline.as_ref().map(|t| t.progress));
                                let total_duration = format_duration(&current_state.timeline.as_ref().map(|t| t.duration));
                                let image_info = if current_state.artwork_data.is_some() { "con imagen" } else { "sin imagen" };
                                
                                info!("Reproduciendo: {} - {} [{}/{}] - {} - {}", 
                                    current_state.title, 
                                    current_state.artist,
                                    progress_duration,
                                    total_duration,
                                    current_state.player_name,
                                    image_info);
                                    
                                log_counter = 0;
                            }
                            
                            debug!("Estado de reproductor actualizado: {} - {}", 
                                   current_state.title, current_state.artist);
                            
                            // Enviamos el nuevo estado
                            if let Err(e) = tx.send(current_state.clone()).await {
                                error!("Error enviando estado del reproductor: {}", e);
                                break;
                            }
                            
                            // Actualizamos el último estado
                            last_state = Some(current_state);
                        } else if log_counter >= log_frequency {
                            // Formatear la información de imagen y duración
                            let image_info = if current_state.artwork_data.is_some() {
                                "con imagen"
                            } else {
                                "sin imagen"
                            };
                            
                            let progress_duration = format_duration(&current_state.timeline.as_ref().map(|t| t.progress));
                            let total_duration = format_duration(&current_state.timeline.as_ref().map(|t| t.duration));
                            
                            info!("Reproduciendo: {} - {} [{}/{}] - {} - {}", 
                                 current_state.title, 
                                 current_state.artist,
                                 progress_duration,
                                 total_duration,
                                 current_state.player_name,
                                 image_info);
                            
                            // También enviamos el estado actualizado para actualizar la posición de reproducción
                            if let Err(e) = tx.send(current_state.clone()).await {
                                error!("Error enviando actualización de posición: {}", e);
                                break;
                            }
                            
                            log_counter = 0;
                        } else {
                            // Enviamos actualizaciones silenciosas cada 5 segundos para mantener la posición actualizada
                            if log_counter % 5 == 0 {
                                if let Err(e) = tx.send(current_state.clone()).await {
                                    error!("Error enviando actualización silenciosa: {}", e);
                                    break;
                                }
                            }
                        }
                    },
                    None => {
                        // Si antes había un estado activo pero ahora no
                        if last_state.is_some() {
                            debug!("Reproductor detenido");
                            
                            // No hay estado activo, enviamos estado de pausa
                            let paused_state = State::paused();
                            if let Err(e) = tx.send(paused_state).await {
                                error!("Error enviando estado de pausa: {}", e);
                                break;
                            }
                            
                            last_state = None;
                            
                            // Reiniciamos el contador para log y track ID
                            log_counter = 0;
                            last_track_id = None;
                        } else if log_counter >= log_frequency {
                            info!("No se detectó ningún reproductor activo");
                            
                            // Enviamos estado de pausa regularmente aunque no haya habido cambios
                            let paused_state = State::paused();
                            if let Err(e) = tx.send(paused_state).await {
                                error!("Error enviando recordatorio de pausa: {}", e);
                                break;
                            }
                            
                            log_counter = 0;
                        }
                    }
                }
                
                tokio::time::sleep(interval).await;
            }
        })
    }
    
    /// Obtiene la portada para Music sin modificar el resto del estado
    fn get_music_artwork(&self, mut state: State) -> State {
        // Obtener la portada con método directo para streaming
        let artwork_script = r#"
            on run
                tell application "Music"
                    try
                        if player state is playing then
                            set currentTrack to current track
                            set artworkPath to "/tmp/music_artwork.jpg"
                            
                            -- Limpiar el archivo anterior si existe
                            do shell script "rm -f " & quoted form of artworkPath
                            
                            -- Enfoque directo usando osascript inline para extraer 
                            -- y guardar la portada, evitando problemas de tipo de pista
                            set hasArtwork to false
                            
                            if exists artwork 1 of currentTrack then
                                -- Crear un script específico para extraer la portada
                                set scriptContent to "tell application \"Music\"
                                    tell artwork 1 of current track
                                        set artData to raw data
                                        set fileRef to open for access POSIX file \"" & artworkPath & "\" with write permission
                                        set eof fileRef to 0
                                        write artData to fileRef
                                        close access fileRef
                                    end tell
                                end tell"
                                
                                -- Guardar el script temporalmente
                                set scriptPath to "/tmp/extract_artwork.scpt"
                                do shell script "cat > " & quoted form of scriptPath & " << 'EOF'
" & scriptContent & "
EOF"
                                
                                -- Ejecutar el script
                                do shell script "osascript " & quoted form of scriptPath & " || true"
                                
                                -- Verificar si el archivo existe y no está vacío
                                set checkCmd to "[ -s " & quoted form of artworkPath & " ] && echo 'OK' || echo 'FAIL'"
                                set checkResult to do shell script checkCmd
                                
                                if checkResult is "OK" then
                                    -- Verificar el tipo de archivo para confirmar que es una imagen
                                    set fileTypeCmd to "file -b " & quoted form of artworkPath
                                    set fileType to do shell script fileTypeCmd
                                    
                                    if fileType contains "image" or fileType contains "JPEG" or fileType contains "PNG" then
                                        return "TEMP_FILE:" & artworkPath
                                    else
                                        log "Archivo generado no es una imagen: " & fileType
                                    end if
                                else
                                    log "No se pudo extraer la portada (archivo vacío)"
                                end if
                            else
                                log "Esta pista no tiene portada"
                            end if
                        end if
                    on error errMsg
                        log "Error obteniendo portada: " & errMsg
                        return "ERROR:" & errMsg
                    end try
                end tell
                return ""
            end run
        "#;

        let artwork_data = match run_applescript(artwork_script) {
            Ok(output) => {
                if output.starts_with("TEMP_FILE:") {
                    let file_path = output.trim_start_matches("TEMP_FILE:");
                    match std::fs::read(file_path) {
                        Ok(data) if !data.is_empty() => {
                            info!("Portada obtenida de Music para {} - {}: {} bytes", 
                                state.title, state.artist, data.len());
                            
                            // Dump de los primeros bytes para diagnóstico
                            let hex_dump = data.iter().take(32)
                                .map(|b| format!("{:02X}", b))
                                .collect::<Vec<_>>()
                                .join(" ");
                            
                            info!("Primeros 32 bytes de la imagen: {}", hex_dump);
                            
                            // Verificar que hay suficientes datos para hacer las comprobaciones de cabecera
                            if data.len() < 4 {
                                debug!("Datos de imagen demasiado pequeños para verificar formato de Music");
                                None
                            }
                            // Asegurarnos de que es una imagen JPEG válida
                            else if data[0] == 0xFF && data[1] == 0xD8 {
                                info!("Portada válida JPEG detectada: {} bytes", data.len());
                                Some(data)
                            } 
                            // O un PNG válido
                            else if data[0] == 0x89 && data[1] == 0x50 && data[2] == 0x4E && data[3] == 0x47 {
                                info!("Portada válida PNG detectada: {} bytes", data.len());
                                Some(data)
                            }
                            else {
                                warn!("Datos de imagen en formato incorrecto de Music. Primeros bytes: {}", hex_dump);
                                None
                            }
                        },
                        Ok(empty_data) => {
                            warn!("Archivo de imagen vacío para Music: {} bytes", empty_data.len());
                            None
                        },
                        Err(e) => {
                            warn!("Error leyendo archivo temporal de imagen: {}", e);
                            None
                        }
                    }
                } else if output.starts_with("ERROR:") {
                    let error = output.trim_start_matches("ERROR:");
                    warn!("Error en AppleScript al obtener portada: {}", error);
                    None
                } else if output.is_empty() {
                    debug!("No se pudo obtener la portada de Music (respuesta vacía)");
                    None
                } else {
                    // Si no tiene los marcadores, intentamos usar los datos directamente
                    let hex_dump = output.as_bytes().iter().take(32)
                        .map(|b| format!("{:02X}", b))
                        .collect::<Vec<_>>()
                        .join(" ");
                    
                    info!("Portada obtenida de Music en formato directo: {} bytes. Primeros bytes: {}", 
                          output.len(), hex_dump);
                    
                    // Verificar si parece una imagen válida
                    let bytes = output.as_bytes();
                    if bytes.len() >= 4 && (
                        // JPEG
                        (bytes[0] == 0xFF && bytes[1] == 0xD8) ||
                        // PNG
                        (bytes[0] == 0x89 && bytes[1] == 0x50 && bytes[2] == 0x4E && bytes[3] == 0x47)
                    ) {
                        info!("Formato de imagen válido en respuesta directa");
                        Some(output.into_bytes())
                    } else {
                        warn!("Formato de imagen inválido en respuesta directa");
                        None
                    }
                }
            },
            Err(e) => {
                warn!("Error obteniendo portada de Music: {}", e);
                None
            }
        };
        
        state.artwork_data = artwork_data;
        state
    }
    
    /// Obtiene la portada para Spotify sin modificar el resto del estado
    fn get_spotify_artwork(&self, mut state: State) -> State {
        // Intento de obtener la portada de Spotify
        let artwork_script = r#"
            on run
                set theData to ""
                tell application "Spotify"
                    try
                        if player state is playing then
                            set currentTrack to current track
                            
                            -- Intenta obtener la portada usando un método alternativo
                            tell application "System Events"
                                set processExists to exists process "Spotify"
                                if processExists then
                                    tell process "Spotify"
                                        -- Intentar capturar el elemento de la portada
                                        -- (Este enfoque puede no funcionar directamente)
                                        tell application "System Events" to keystroke "i" using {command down, shift down}
                                        delay 0.5
                                        -- Copiar al portapapeles
                                        tell application "System Events" to keystroke "c" using {command down}
                                        
                                        -- Volver a la vista normal
                                        tell application "System Events" to keystroke "i" using {command down, shift down}
                                        
                                        -- Contenido del portapapeles
                                        set theData to get the clipboard
                                    end tell
                                end if
                            end tell
                        end if
                    on error errMsg
                        log "Error obteniendo portada de Spotify: " & errMsg
                    end try
                end tell
                
                return theData
            end run
        "#;

        // Intentar obtener la portada, aunque esto puede fallar con Spotify
        let artwork_data = match run_applescript(artwork_script) {
            Ok(output) => {
                if output.is_empty() {
                    debug!("No se pudo obtener la portada de Spotify (limitación de API)");
                    None
                } else {
                    info!("Portada obtenida de Spotify para {} - {}: {} bytes", 
                         state.title, state.artist, output.len());
                    Some(output.into_bytes())
                }
            },
            Err(e) => {
                debug!("Error intentando obtener portada de Spotify: {}", e);
                None
            }
        };
        
        state.artwork_data = artwork_data;
        state
    }
}

/// Determina si el estado actual debe actualizar el estado anterior
fn should_update_state(last_state: &Option<State>, current_state: &State) -> bool {
    if let Some(last) = last_state {
        // Verificamos si alguno de los campos importantes ha cambiado
        last.title != current_state.title ||
        last.artist != current_state.artist ||
        last.player_name != current_state.player_name ||
        // Si la línea de tiempo ha cambiado significativamente (más de 5 segundos)
        (match (&last.timeline, &current_state.timeline) {
            (Some(last_tl), Some(cur_tl)) => {
                let diff = if last_tl.progress > cur_tl.progress {
                    last_tl.progress - cur_tl.progress
                } else {
                    cur_tl.progress - last_tl.progress
                };
                diff > Duration::from_secs(5)
            }
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        })
    } else {
        // Si no hay estado anterior, siempre actualizamos
        true
    }
}

/// Ejecutar un comando AppleScript
pub fn run_applescript(script: &str) -> Result<String, Error> {
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| Error::AppleScript(e.to_string()))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(Error::AppleScript(stderr))
    }
}

/// Ejecutar un comando AppleScript de forma asíncrona
pub async fn run_applescript_async(script: &str) -> Result<String, Error> {
    let output = tokio::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .await
        .map_err(|e| Error::AppleScript(e.to_string()))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(Error::AppleScript(stderr))
    }
}

/// Formatea una duración en formato MM:SS
fn format_duration(duration: &Option<Duration>) -> String {
    if let Some(d) = duration {
        let total_secs = d.as_secs();
        if total_secs == 0 {
            return "--:--".to_string();
        }
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        format!("{:02}:{:02}", mins, secs)
    } else {
        "--:--".to_string()
    }
} 