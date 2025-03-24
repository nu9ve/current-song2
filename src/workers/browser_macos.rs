use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix::Addr;
use tokio::process::Command;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{debug, error, info};

use crate::actors::manager::{Manager, UpdateModule};
use crate::config::BrowserMacOsConfig;
use crate::image_store::{ImageStore, SlotRef};
use crate::model::{ModuleState, PlayInfo, TimelineInfo};

const POLL_INTERVAL: Duration = Duration::from_secs(3);

// Estructura para almacenar información sobre una pestaña de navegador
#[derive(Debug, Clone, PartialEq)]
struct BrowserTab {
    title: String,
    url: String,
    browser_name: String,
}

// Implementación para determinar si la URL pertenece a un servicio de música
impl BrowserTab {
    fn get_music_info(&self) -> Option<(String, String, String)> {
        // Extraer información de título basada en el título de la pestaña
        let title = self.title.clone();
        debug!("Analizando pestaña: '{}' | URL: '{}'", title, self.url);
        
        // YouTube Music: "Nombre canción - Artista - YouTube Music"
        if self.url.contains("music.youtube.com") {
            if let Some(idx) = title.rfind(" - YouTube Music") {
                let content = &title[0..idx];
                if let Some(separator) = content.rfind(" - ") {
                    let song_title = content[0..separator].trim().to_string();
                    let artist = content[separator + 3..].trim().to_string();
                    debug!("Extraído de YouTube Music: '{}' - '{}'", song_title, artist);
                    return Some((song_title, artist, "youtube-music".to_string()));
                }
            }
            // Formato alternativo
            return self.extract_generic_title_dash_artist(title, "YouTube Music");
        }
        
        // YouTube: Ahora siempre extraeremos el título completo primero
        if self.url.contains("youtube.com/watch") {
            // Primero extraer el título de la pestaña sin sufijos
            let clean_title = if title.contains(" - YouTube") {
                title.replace(" - YouTube", "").trim().to_string()
            } else {
                title.clone()
            };
            
            // Eliminar prefijos como "(78)" que suelen aparecer en YouTube
            let clean_title = if clean_title.starts_with('(') {
                if let Some(close_idx) = clean_title.find(')') {
                    if close_idx < 10 { // Solo si es un prefijo corto
                        clean_title[close_idx + 1..].trim().to_string()
                    } else {
                        clean_title
                    }
                } else {
                    clean_title
                }
            } else {
                clean_title
            };
            
            debug!("YouTube: Usando título de pestaña limpio: '{}'", clean_title);
            
            // Intentar extraer artista y título si el formato es "Título - Artista"
            if let Some(separator) = clean_title.rfind(" - ") {
                let song_title = clean_title[0..separator].trim().to_string();
                let artist = clean_title[separator + 3..].trim().to_string();
                
                debug!("YouTube: Extrayendo título y artista: '{}' - '{}'", song_title, artist);
                return Some((song_title, artist, "youtube".to_string()));
            }
            
            // Si no se pudo extraer un formato estándar, usar el título completo
            debug!("YouTube: Usando título completo: '{}'", clean_title);
            return Some((clean_title, "YouTube".to_string(), "youtube".to_string()));
        }
        
        // SoundCloud: Ahora siempre extraeremos primero el título completo de la tab
        if self.url.contains("soundcloud.com") {
            // Primero extraer el título de la pestaña sin sufijos
            let clean_title = if title.contains(" | SoundCloud") {
                title.replace(" | SoundCloud", "").trim().to_string()
            } else if title.contains(" | Stream") {
                title.split(" | Stream").next().unwrap_or(&title).trim().to_string()
            } else {
                title.clone()
            };
            
            debug!("SoundCloud: Usando título de pestaña: '{}'", clean_title);
            
            // Intentar extraer artista y título si el formato es "Artista - Título"
            if let Some(separator) = clean_title.find(" - ") {
                let artist = clean_title[0..separator].trim().to_string();
                let song_title = clean_title[separator + 3..].trim().to_string();
                
                debug!("SoundCloud: Extrayendo artista y título: '{}' - '{}'", song_title, artist);
                return Some((song_title, artist, "soundcloud".to_string()));
            }
            
            // Si no se pudo extraer un formato "Artista - Título", usar el título completo
            debug!("SoundCloud: Usando título completo: '{}'", clean_title);
            return Some((clean_title, "SoundCloud".to_string(), "soundcloud".to_string()));
        }
        
        // Bandcamp: "Nombre canción, by Artista"
        if self.url.contains("bandcamp.com") {
            if let Some(idx) = title.find(", by ") {
                let song_title = title[0..idx].trim().to_string();
                let artist = title[idx + 5..].trim().to_string();
                debug!("Extraído de Bandcamp: '{}' - '{}'", song_title, artist);
                return Some((song_title, artist, "bandcamp".to_string()));
            }
        }
        
        // Spotify Web Player: "Spotify Web Player: Nombre canción - Artista"
        if self.url.contains("open.spotify.com") {
            if title.starts_with("Spotify Web Player: ") {
                let content = &title["Spotify Web Player: ".len()..];
                if let Some(separator) = content.rfind(" - ") {
                    let song_title = content[0..separator].trim().to_string();
                    let artist = content[separator + 3..].trim().to_string();
                    debug!("Extraído de Spotify: '{}' - '{}'", song_title, artist);
                    return Some((song_title, artist, "spotify-web".to_string()));
                }
            }
            
            // Si solo obtenemos "Spotify Web Player", intentar extraer de la URL
            if title == "Spotify Web Player" && self.url.contains("/track/") {
                let track_name = "Spotify Track";  // No podemos extraer mejor sin JavaScript
                let artist = "Spotify";
                return Some((track_name.to_string(), artist.to_string(), "spotify-web".to_string()));
            }
        }
        
        // Método genérico para extraer título y artista de URLs desconocidas
        if self.url.contains("music") || 
           self.url.contains("audio") || 
           self.url.contains("player") {
            // Intentar formato genérico "Título - Artista"
            return self.extract_generic_title_dash_artist(title, "");
        }
        
        None
    }
    
    // Extrae información en formato "Título - Artista" quitando sufijos
    fn extract_generic_title_dash_artist(&self, title: String, suffix_to_remove: &str) -> Option<(String, String, String)> {
        let clean_title = if !suffix_to_remove.is_empty() && title.ends_with(suffix_to_remove) {
            let idx = title.rfind(suffix_to_remove).unwrap_or(title.len());
            // Eliminar el sufijo y cualquier guión, barra vertical o texto entre paréntesis
            title[0..idx].trim().trim_end_matches(" -").trim_end_matches(" |").trim().to_string()
        } else {
            title
        };
        
        if let Some(separator) = clean_title.rfind(" - ") {
            let song_title = clean_title[0..separator].trim().to_string();
            let artist = clean_title[separator + 3..].trim().to_string();
            
            // Comprobar que tanto título como artista no estén vacíos
            if !song_title.is_empty() && !artist.is_empty() {
                let provider = if !suffix_to_remove.is_empty() {
                    suffix_to_remove.to_lowercase().replace(' ', "-")
                } else {
                    format!("browser-{}", self.browser_name.to_lowercase())
                };
                
                debug!("Extraído genéricamente: '{}' - '{}' ({})", song_title, artist, provider);
                return Some((song_title, artist, provider));
            }
        }
        
        // Si no podemos extraer título y artista, pero tiene un título, usamos el título completo
        if !clean_title.is_empty() {
            let provider = if !suffix_to_remove.is_empty() {
                suffix_to_remove.to_lowercase().replace(' ', "-")
            } else {
                format!("browser-{}", self.browser_name.to_lowercase())
            };
            
            debug!("Usando título completo: '{}' ({})", clean_title, provider);
            return Some((clean_title, provider.to_string(), provider));
        }
        
        None
    }
}

pub struct BrowserMacOsWorker {
    manager: Addr<Manager>,
    module_id: usize,
    is_paused: std::sync::atomic::AtomicBool,
    browsers: Vec<String>,
    current_tab: Arc<Mutex<Option<BrowserTab>>>,
    image_store: Arc<RwLock<ImageStore>>,
    image_id: SlotRef,
    last_poll_time: Arc<Mutex<Instant>>,
}

pub async fn start_spawning(
    config: BrowserMacOsConfig,
    image_store: Arc<RwLock<ImageStore>>,
    sender: Addr<Manager>,
    module_id: usize,
) -> JoinHandle<()> {
    info!("Iniciando módulo de navegadores para macOS");
    debug!("Configuración: {:?}", config);
    
    let image_id = SlotRef::new(&image_store);
    
    let worker = BrowserMacOsWorker {
        manager: sender,
        module_id,
        is_paused: std::sync::atomic::AtomicBool::new(true),
        browsers: config.browsers,
        current_tab: Arc::new(Mutex::new(None)),
        image_store,
        image_id,
        last_poll_time: Arc::new(Mutex::new(Instant::now())),
    };
    
    // Crear la tarea para sondear los navegadores
    tokio::spawn(async move {
        info!("Iniciando monitoreo de navegadores en macOS");
        
        loop {
            // Sondear pestañas de navegadores
            worker.poll_browsers().await;
            
            // Esperar antes del siguiente sondeo
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
}

impl BrowserMacOsWorker {
    async fn poll_browsers(&self) {
        // Verificar si Apple Music está reproduciendo algo
        let is_apple_music_active = self.check_apple_music_active().await;
        
        // En lugar de mostrar siempre todas las pestañas, solo lo haremos ocasionalmente
        let now = Instant::now();
        let should_list_tabs = {
            let mut last_poll = self.last_poll_time.lock().unwrap();
            let should_list = now.duration_since(*last_poll) > Duration::from_secs(30); // Aumentar a 30 segundos
            if should_list {
                // Actualizar la hora del último sondeo completo
                *last_poll = now;
            }
            should_list
        }; // El MutexGuard se libera aquí al final del bloque
        
        // Solo listar todas las pestañas cada cierto tiempo y cuando no hay Apple Music activo
        if should_list_tabs && !is_apple_music_active {
            self.list_all_media_tabs().await;
        }
        
        // Si Apple Music está activo, pausar la detección del navegador
        if is_apple_music_active {
            // Pausar la reproducción del navegador si estaba reproduciendo
            let was_playing = !self.is_paused.swap(true, std::sync::atomic::Ordering::SeqCst);
            
            if was_playing {
                info!("⏸️ Navegador pausado porque Apple Music está activo");
                
                // Limpiar pestaña actual
                {
                    let mut current_tab = self.current_tab.lock().unwrap();
                    *current_tab = None;
                }
                
                // Enviar estado de pausa
                self.manager.do_send(UpdateModule {
                    id: self.module_id,
                    state: ModuleState::Paused,
                });
            }
            
            return;
        }
        
        // Si llegamos aquí, Apple Music no está activo, continuar con la detección normal
        
        // Detectar pestañas de navegadores con AppleScript
        if let Some(tab) = self.get_active_tab().await {
            debug!("Pestaña detectada: {} | {}", tab.title, tab.url);
            
            // Verificar si la pestaña actual contiene información de música
            if let Some((title, artist, provider)) = tab.get_music_info() {
                // Solo mostrar mensaje de reproducción si cambió la canción o si pasó tiempo suficiente
                let should_log = {
                    let mut current_tab = self.current_tab.lock().unwrap();
                    let tab_changed = current_tab.as_ref().map_or(true, |prev_tab| {
                        prev_tab.title != tab.title || prev_tab.url != tab.url
                    });
                    
                    if tab_changed {
                        *current_tab = Some(tab.clone());
                        true
                    } else {
                        false
                    }
                };
                
                if should_log {
                    info!("📌 Reproduciendo: {} - {} ({})", title, artist, provider);
                }
                
                // Actualizar el estado a reproduciendo
                self.is_paused.store(false, std::sync::atomic::Ordering::SeqCst);
                
                // Crear información de reproducción
                let play_info = self.create_play_info(title, artist, provider);
                
                // Solo mostrar información para diagnóstico si realmente es necesario
                if should_log {
                    debug!("Enviando al widget: título='{}', artista='{}', origen='{}'", 
                        play_info.title, play_info.artist, play_info.source);
                }
                
                // Enviar el estado al gestor
                self.manager.do_send(UpdateModule {
                    id: self.module_id,
                    state: ModuleState::Playing(play_info),
                });
                
                return;
            }
        }
        
        // Si no encontramos pestaña activa con música, verificar si tenemos una pestaña recordada
        // y comprobar si aún está reproduciendo música
        let tab_to_check = {
            let current_tab = self.current_tab.lock().unwrap();
            current_tab.clone()
        };
        
        if let Some(previous_tab) = tab_to_check {
            // Verificar si el navegador y la URL aún existen usando un script AppleScript más básico
            if let Some((title, artist, provider)) = previous_tab.get_music_info() {
                if self.is_tab_still_valid(&previous_tab).await {
                    debug!("Manteniendo: {} - {} ({})", title, artist, provider);
                    
                    // Mantener el estado como reproduciendo
                    let is_already_playing = !self.is_paused.load(std::sync::atomic::Ordering::SeqCst);
                    
                    // Solo enviar actualizaciones si no estábamos ya reproduciendo
                    if !is_already_playing {
                        self.is_paused.store(false, std::sync::atomic::Ordering::SeqCst);
                        
                        // Crear información de reproducción
                        let play_info = self.create_play_info(title, artist, provider);
                        
                        // Mostrar información para diagnóstico
                        debug!("Reactivando: título='{}', artista='{}', origen='{}'", 
                            play_info.title, play_info.artist, play_info.source);
                        
                        // Enviar el estado al gestor
                        self.manager.do_send(UpdateModule {
                            id: self.module_id,
                            state: ModuleState::Playing(play_info),
                        });
                    }
                    
                    return;
                }
            }
        }
        
        // No se encontró información de música actual ni recordada, establecer estado de pausa
        let was_playing = !self.is_paused.swap(true, std::sync::atomic::Ordering::SeqCst);
        
        if was_playing {
            info!("⏸️ No se detectó contenido musical en los navegadores");
            
            // Limpiar pestaña actual
            {
                let mut current_tab = self.current_tab.lock().unwrap();
                *current_tab = None;
            }
            
            // Enviar estado de pausa
            self.manager.do_send(UpdateModule {
                id: self.module_id,
                state: ModuleState::Paused,
            });
        }
    }
    
    async fn get_active_tab(&self) -> Option<BrowserTab> {
        for browser in &self.browsers {
            if let Some(tab) = self.get_browser_active_tab(browser).await {
                return Some(tab);
            }
        }
        
        None
    }
    
    async fn get_browser_active_tab(&self, browser: &str) -> Option<BrowserTab> {
        // Ya no necesitamos verificar Apple Music aquí, ya lo hacemos en poll_browsers
        
        // Primero obtenemos todas las pestañas potenciales de música
        let potential_tabs = self.get_all_media_tabs(browser).await;
        
        if potential_tabs.is_empty() {
            return None;
        }
        
        // Recopilamos información sobre diferentes tipos de pestañas
        let mut unmuted_tabs = Vec::new();
        let mut muted_tabs = Vec::new();
        let mut likely_audio_tabs = Vec::new();
        let mut media_tabs = Vec::new();
        let mut active_tabs = Vec::new();
        
        // Estas son listas específicas para cada servicio de música
        let mut soundcloud_tabs = Vec::new();
        let mut youtube_tabs = Vec::new();
        let mut youtube_music_tabs = Vec::new();
        let mut spotify_tabs = Vec::new();
        
        // Analizar cada pestaña para determinar su estado
        for tab in &potential_tabs {
            // Clasificar por servicios específicos primero (esto es independiente del estado de audio)
            if tab.url.contains("soundcloud.com") {
                soundcloud_tabs.push(tab.clone());
            } else if tab.url.contains("youtube.com/watch") {
                youtube_tabs.push(tab.clone());
            } else if tab.url.contains("music.youtube.com") {
                youtube_music_tabs.push(tab.clone());
            } else if tab.url.contains("open.spotify.com") {
                spotify_tabs.push(tab.clone());
            }
            
            // Ahora clasificamos por estado de audio
            let audio_state = self.check_tab_audio_state(browser, &tab.url).await;
            
            match audio_state.as_str() {
                "true-unmuted" => {
                    debug!("✅ Audio NO MUTEADO: {} | {}", tab.title, tab.url);
                    unmuted_tabs.push(tab.clone());
                },
                "true-muted" => {
                    debug!("🔇 Audio MUTEADO: {} | {}", tab.title, tab.url);
                    muted_tabs.push(tab.clone());
                },
                "true-likely" => {
                    debug!("🎵 Probable audio: {} | {}", tab.title, tab.url);
                    likely_audio_tabs.push(tab.clone());
                },
                "true-media" => {
                    debug!("📱 Contenido multimedia: {} | {}", tab.title, tab.url);
                    media_tabs.push(tab.clone());
                },
                "true-active" => {
                    debug!("👁 Pestaña activa: {} | {}", tab.title, tab.url);
                    active_tabs.push(tab.clone());
                },
                _ => {} // No loggear pestañas sin audio
            }
        }
        
        // ALGORITMO DE PRIORIZACIÓN:
        
        // 1. Máxima prioridad: Pestañas con audio no muteado
        if !unmuted_tabs.is_empty() {
            // Entre las pestañas con audio, priorizar SoundCloud
            if let Some(tab) = unmuted_tabs.iter().find(|t| t.url.contains("soundcloud.com")) {
                return Some(tab.clone());
            }
            
            // Luego priorizar YouTube
            if let Some(tab) = unmuted_tabs.iter().find(|t| t.url.contains("youtube.com/watch")) {
                return Some(tab.clone());
            }
            
            // Si no hay ninguna de las anteriores, usar cualquier pestaña con audio no muteado
            return Some(unmuted_tabs[0].clone());
        }
        
        // 2. Segunda prioridad: Pestañas con audio muteado, con la misma subpriorización
        if !muted_tabs.is_empty() {
            // Entre las pestañas muteadas, priorizar SoundCloud
            if let Some(tab) = muted_tabs.iter().find(|t| t.url.contains("soundcloud.com")) {
                return Some(tab.clone());
            }
            
            // Luego priorizar YouTube
            if let Some(tab) = muted_tabs.iter().find(|t| t.url.contains("youtube.com/watch")) {
                return Some(tab.clone());
            }
            
            return Some(muted_tabs[0].clone());
        }
        
        // 3. Priorizar por servicio específico (SoundCloud > YouTube > otras)
        
        // Primero SoundCloud (mayor prioridad)
        if !soundcloud_tabs.is_empty() {
            // Preferir la última pestaña (generalmente la más reciente)
            let selected_tab = soundcloud_tabs.last().unwrap();
            return Some(selected_tab.clone());
        }
        
        // Luego YouTube (segunda prioridad)
        if !youtube_tabs.is_empty() {
            // Preferir la última pestaña (generalmente la más reciente)
            let selected_tab = youtube_tabs.last().unwrap();
            return Some(selected_tab.clone());
        }
        
        // Luego YouTube Music
        if !youtube_music_tabs.is_empty() {
            let selected_tab = youtube_music_tabs.last().unwrap();
            return Some(selected_tab.clone());
        }
        
        // Luego Spotify
        if !spotify_tabs.is_empty() {
            let selected_tab = spotify_tabs.last().unwrap();
            return Some(selected_tab.clone());
        }
        
        // 4. Continuar con las prioridades para pestañas con probable audio
        if !likely_audio_tabs.is_empty() {
            return Some(likely_audio_tabs[0].clone());
        }
        
        if !media_tabs.is_empty() {
            return Some(media_tabs[0].clone());
        }
        
        if !active_tabs.is_empty() {
            return Some(active_tabs[0].clone());
        }
        
        // 5. Como último recurso, usar la primera pestaña encontrada
        if !potential_tabs.is_empty() {
            return Some(potential_tabs[0].clone());
        }
        
        None
    }
    
    // Verificar el estado de audio de una pestaña y retornar su estado
    async fn check_tab_audio_state(&self, browser: &str, tab_url: &str) -> String {
        // Script mejorado para detectar reproducción de audio, especialmente en Brave/Chrome
        let script = format!(
            r#"
            tell application "System Events"
                set browserRunning to application process "{}" exists
            end tell
            
            if browserRunning then
                tell application "{}"
                    if it is running then
                        set windowCount to count of windows
                        
                        if windowCount > 0 then
                            repeat with windowIndex from 1 to windowCount
                                set currentWindow to window windowIndex
                                
                                try
                                    set tabCount to count of tabs of currentWindow
                                    
                                    if tabCount > 0 then
                                        repeat with tabIndex from 1 to tabCount
                                            set currentTab to tab tabIndex of currentWindow
                                            
                                            try
                                                if URL of currentTab contains "{}" then
                                                    -- JavaScript específico para detectar reproducción de audio en Chromium
                                                    set jsResult to execute currentTab javascript "
                                                        (function() {{
                                                            // Analizar la página para detectar elementos multimedia
                                                            
                                                            // 1. Comprobar paneles de reproducción multimedia específicos de Brave/Chrome
                                                            // Estos elementos aparecen en el mini-reproductor que se muestra para los medios activos
                                                            const mediaControls = document.querySelector('#media-button.media-button--playing, .ytp-miniplayer-button[aria-pressed=\"true\"]');
                                                            if (mediaControls) {{
                                                                // Si encontramos controles de reproducción activos, verificamos si está muteado
                                                                const volumeButton = document.querySelector('.ytp-mute-button[aria-label*=\"Unmute\"], button[aria-label*=\"activar el sonido\"], button[title*=\"Unmute\"]');
                                                                if (volumeButton) {{
                                                                    return 'playing-muted'; // Reproduciendo pero muteado
                                                                }} else {{
                                                                    return 'playing-unmuted'; // Reproduciendo con sonido
                                                                }}
                                                            }}
                                                            
                                                            // 2. Detectar elementos <audio> o <video> estándar que estén reproduciendo
                                                            const mediaElements = Array.from(document.querySelectorAll('audio, video'));
                                                            for (const element of mediaElements) {{
                                                                // Verificar si el elemento está reproduciendo y no está en pausa
                                                                if (!element.paused && element.currentTime > 0) {{
                                                                    if (element.muted || element.volume === 0) {{
                                                                        return 'playing-muted';
                                                                    }} else {{
                                                                        return 'playing-unmuted';
                                                                    }}
                                                                }}
                                                            }}
                                                            
                                                            // 3. Casos específicos para diferentes plataformas
                                                            
                                                            // YouTube
                                                            if (window.location.href.includes('youtube.com')) {{
                                                                // Verificar si hay botón de pausa (lo que indica reproducción activa)
                                                                const pauseButton = document.querySelector('button[aria-label*=\"Pause\"], button[aria-label*=\"pausar\"], button[aria-label*=\"pausa\"]');
                                                                if (pauseButton) {{
                                                                    // Está reproduciendo, verificar si está muteado
                                                                    const muteButton = document.querySelector('button[aria-label*=\"Unmute\"], button[aria-label*=\"activar\"], button[title*=\"Unmute\"]');
                                                                    const volumeSlider = document.querySelector('.ytp-volume-slider-handle');
                                                                    const volumePanel = document.querySelector('.ytp-volume-panel');
                                                                    
                                                                    if (muteButton || (volumeSlider && volumeSlider.style.left === '0px') || (volumePanel && volumePanel.getAttribute('aria-valuenow') === '0')) {{
                                                                        return 'playing-muted';
                                                                    }} else {{
                                                                        return 'playing-unmuted';
                                                                    }}
                                                                }}
                                                            }}
                                                            
                                                            // SoundCloud
                                                            if (window.location.href.includes('soundcloud.com')) {{
                                                                const playButton = document.querySelector('.playControl.playing');
                                                                if (playButton) {{
                                                                    const volumeButton = document.querySelector('.volume__button.muted');
                                                                    if (volumeButton || document.querySelector('.volume__sliderProgress[style*=\"width: 0%\"]')) {{
                                                                        return 'playing-muted';
                                                                    }} else {{
                                                                        return 'playing-unmuted';
                                                                    }}
                                                                }}
                                                            }}
                                                            
                                                            // 4. Último recurso: buscar indicadores visuales en el DOM
                                                            // Buscar elementos que suelen indicar reproducción activa
                                                            const playingIndicators = document.querySelectorAll('.playing, [class*=\"playing\"], [data-playing=\"true\"], [aria-label*=\"playing\"]');
                                                            if (playingIndicators.length > 0) {{
                                                                // Encontramos algún indicador de reproducción, asumimos que está reproduciendo 
                                                                // pero no podemos determinar si está muteado
                                                                return 'playing-likely';
                                                            }}
                                                            
                                                            // 5. Verificar si la página tiene el título cambiado dinámicamente (común en reproductores)
                                                            // Esto puede ayudar a detectar reproductores web personalizados
                                                            if (document.title.includes(' - ') && 
                                                                (document.title.includes('YouTube') || 
                                                                 document.title.includes('SoundCloud') || 
                                                                 document.title.includes('Music'))) {{
                                                                return 'likely-media';
                                                            }}
                                                            
                                                            // No se encontró reproducción activa
                                                            return 'not-playing';
                                                        }})()
                                                    "
                                                    
                                                    if jsResult is "playing-unmuted" then
                                                        return "true-unmuted"
                                                    else if jsResult is "playing-muted" then
                                                        return "true-muted"
                                                    else if jsResult is "playing-likely" then
                                                        return "true-likely"
                                                    else if jsResult is "likely-media" then
                                                        return "true-media"
                                                    else
                                                        -- Verificar si es la pestaña activa
                                                        if currentTab is active tab of currentWindow then
                                                            return "true-active"
                                                        end if
                                                    end if
                                                end if
                                            end if
                                        on error errMsg
                                            -- Error al evaluar JavaScript
                                            log "Error JavaScript: " & errMsg
                                        end try
                                    end repeat
                                end if
                            on error
                                -- Error al acceder a las pestañas
                            end try
                        end repeat
                    end if
                end tell
            end if
            
            return "false"
            "#, 
            browser, browser, tab_url
        );
        
        let output = match Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .await 
        {
            Ok(output) => {
                if output.status.success() {
                    String::from_utf8_lossy(&output.stdout).to_string().trim().to_string()
                } else {
                    debug!("Error al verificar audio: {:?}", output.stderr);
                    "false".to_string()
                }
            },
            Err(e) => {
                error!("Error al ejecutar osascript para verificar audio: {}", e);
                "false".to_string()
            }
        };
        
        output
    }
    
    fn create_play_info(&self, title: String, artist: String, provider: String) -> PlayInfo {
        // Crear timestamp para la línea de tiempo
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
            
        // Como no podemos determinar la duración, usamos una línea de tiempo básica
        let timeline = TimelineInfo {
            progress_ms: 0,
            duration_ms: 300000,  // 5 minutos como duración ficticia para que el widget muestre algo
            ts: now,
            rate: 1.0,
        };
        
        // Formatear el origen de manera consistente para que el widget pueda mostrar el icono correcto
        let source = if provider == "soundcloud" {
            "browser-soundcloud".to_string()
        } else if provider == "youtube" {
            "browser-youtube".to_string()
        } else if provider == "youtube-music" {
            "browser-youtube-music".to_string()
        } else if provider == "spotify-web" {
            "browser-spotify".to_string()
        } else {
            format!("browser-{}", provider.to_lowercase())
        };
        
        // Asegúrate de que el título y el artista no estén vacíos
        let final_title = if title.trim().is_empty() { "Desconocido".to_string() } else { title };
        let final_artist = if artist.trim().is_empty() { "Desconocido".to_string() } else { artist };
        
        info!("💿 Creando información de reproducción: '{}' - '{}' | Origen: '{}'", 
              final_title, final_artist, source);
        
        PlayInfo {
            title: final_title,
            artist: final_artist,
            source,
            album: None,
            track_number: None,
            timeline: Some(timeline),
            image: None,  // No tenemos imagen por ahora
        }
    }

    // Verificar si una pestaña recordada sigue siendo válida (navegador abierto y URL existe)
    async fn is_tab_still_valid(&self, tab: &BrowserTab) -> bool {
        let script = format!(
            r#"
            tell application "System Events"
                set browserRunning to application process "{}" exists
            end tell
            
            if browserRunning then
                tell application "{}"
                    if it is running then
                        set windowCount to count of windows
                        
                        if windowCount > 0 then
                            repeat with windowIndex from 1 to windowCount
                                set currentWindow to window windowIndex
                                
                                try
                                    set tabCount to count of tabs of currentWindow
                                    
                                    if tabCount > 0 then
                                        repeat with tabIndex from 1 to tabCount
                                            set currentTab to tab tabIndex of currentWindow
                                            
                                            try
                                                if URL of currentTab is "{}" then
                                                    return "true"
                                                end if
                                            on error
                                                -- Continuar si no podemos leer la URL
                                            end try
                                        end repeat
                                    end if
                                on error
                                    -- Continuar si no podemos leer las pestañas
                                end try
                            end repeat
                        end if
                    end if
                end tell
            end if
            
            return "false"
            "#, 
            tab.browser_name, tab.browser_name, tab.url
        );
        
        let output = match Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .await 
        {
            Ok(output) => {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string().trim().to_string();
                    stdout == "true"
                } else {
                    debug!("Error al verificar pestaña: {:?}", output.stderr);
                    false
                }
            },
            Err(e) => {
                error!("Error al ejecutar osascript para verificar pestaña: {}", e);
                false
            }
        };
        
        output
    }

    // Listar todas las pestañas con posible contenido multimedia de manera más compacta
    async fn list_all_media_tabs(&self) {
        info!("📋 Revisando pestañas con posible contenido multimedia");
        
        let mut total_tabs = 0;
        let mut media_tabs_count = 0;
        
        // Contamos todas las pestañas y solo mostramos las que tienen contenido de música
        for browser in &self.browsers {
            let tabs = self.get_all_media_tabs(browser).await;
            total_tabs += tabs.len();
            
            if !tabs.is_empty() {
                let media_tabs: Vec<&BrowserTab> = tabs.iter()
                    .filter(|tab| tab.get_music_info().is_some())
                    .collect();
                
                media_tabs_count += media_tabs.len();
                
                // Solo mostrar el resumen si hay pestañas con contenido musical
                if !media_tabs.is_empty() {
                    debug!("  {} ({} pestañas con contenido musical)", browser, media_tabs.len());
                    
                    // Mostrar máximo 5 pestañas para no saturar los logs
                    for tab in media_tabs.iter().take(5) {
                        if let Some((title, artist, _)) = tab.get_music_info() {
                            debug!("  → {} - {} | {}", title, artist, tab.url);
                        }
                    }
                    
                    // Indicar si hay más pestañas
                    if media_tabs.len() > 5 {
                        debug!("  → ... y {} más", media_tabs.len() - 5);
                    }
                }
            }
        }
        
        debug!("Total: {} pestañas detectadas, {} con contenido musical", total_tabs, media_tabs_count);
    }
    
    // Obtener todas las pestañas con posible contenido multimedia en un navegador
    async fn get_all_media_tabs(&self, browser: &str) -> Vec<BrowserTab> {
        let script = format!(
            r#"
            tell application "System Events"
                set browserRunning to application process "{}" exists
            end tell
            
            set mediaResults to ""
            
            if browserRunning then
                tell application "{}"
                    if it is running then
                        set windowCount to count of windows
                        
                        if windowCount > 0 then
                            repeat with windowIndex from 1 to windowCount
                                set currentWindow to window windowIndex
                                
                                try
                                    set tabCount to count of tabs of currentWindow
                                    
                                    if tabCount > 0 then
                                        repeat with tabIndex from 1 to tabCount
                                            set currentTab to tab tabIndex of currentWindow
                                            
                                            try
                                                set currentURL to URL of currentTab
                                                set pageTitle to title of currentTab
                                                
                                                -- Verificar si la pestaña es potencialmente de música
                                                if currentURL contains "youtube.com/watch" or currentURL contains "music.youtube.com" or currentURL contains "soundcloud.com" or currentURL contains "bandcamp.com" or currentURL contains "open.spotify.com" or currentURL contains "music" or currentURL contains "audio" or currentURL contains "player" then
                                                    set mediaResults to mediaResults & pageTitle & "|" & currentURL & "\n"
                                                end if
                                            on error
                                                -- Error al obtener URL o título
                                            end try
                                        end repeat
                                    end if
                                on error
                                    -- Error al acceder a las pestañas
                                end try
                            end repeat
                        end if
                    end if
                end tell
            end if
            
            return mediaResults
            "#, 
            browser, browser
        );
        
        let output = match Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .await 
        {
            Ok(output) => {
                if output.status.success() {
                    String::from_utf8_lossy(&output.stdout).to_string()
                } else {
                    debug!("Error al ejecutar AppleScript: {:?}", output.stderr);
                    String::new()
                }
            },
            Err(e) => {
                error!("Error al ejecutar osascript: {}", e);
                String::new()
            }
        };
        
        let mut tabs = Vec::new();
        
        // Procesar las líneas para extraer pestañas
        for line in output.trim().lines() {
            if line.contains('|') {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() >= 2 {
                    let title = parts[0].trim();
                    let url = parts[1].trim();
                    
                    if !title.is_empty() && !url.is_empty() {
                        tabs.push(BrowserTab {
                            title: title.to_string(),
                            url: url.to_string(),
                            browser_name: browser.to_string(),
                        });
                    }
                }
            }
        }
        
        tabs
    }

    // Función para verificar si Apple Music está activo
    async fn check_apple_music_active(&self) -> bool {
        let script = r#"
        tell application "System Events"
            set musicAppRunning to application process "Music" exists
        end tell
        
        if musicAppRunning then
            tell application "Music"
                if it is running then
                    if player state is playing then
                        return "true"
                    end if
                end if
            end tell
        end if
        
        return "false"
        "#;
        
        let output = match Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .await 
        {
            Ok(output) => {
                if output.status.success() {
                    String::from_utf8_lossy(&output.stdout).to_string().trim().to_string() == "true"
                } else {
                    debug!("Error al verificar Apple Music: {:?}", output.stderr);
                    false
                }
            },
            Err(e) => {
                error!("Error al ejecutar osascript para verificar Apple Music: {}", e);
                false
            }
        };
        
        output
    }
} 