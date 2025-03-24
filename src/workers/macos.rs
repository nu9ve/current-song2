use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use actix::Addr;
use macos_media::player::{self, State};
use tokio::task::JoinHandle;
use tokio::time::{self, Instant};
use tracing::{debug, info, warn};

use crate::actors::manager::{Manager, UpdateModule};
use crate::config::MacOsConfig;
use crate::image_store::{ImageStore, SlotRef};
use crate::model::{AlbumInfo, InternalImage, ImageInfo, ModuleState, PlayInfo, TimelineInfo};

pub struct MacOsWorker {
    manager: Addr<Manager>,
    module_id: usize,
    is_paused: std::sync::atomic::AtomicBool,
    player_name: Option<String>,
    image_store: Arc<RwLock<ImageStore>>,
    image_id: SlotRef,
    current_track_id: Arc<Mutex<Option<String>>>,
    last_log_time: Arc<Mutex<Instant>>,
}

pub async fn start_spawning(
    config: MacOsConfig,
    image_store: Arc<RwLock<ImageStore>>,
    sender: Addr<Manager>,
    module_id: usize,
) -> JoinHandle<()> {
    info!("Iniciando módulo macOS");
    debug!("Configuración: {:?}", config);
    
    let image_id = SlotRef::new(&image_store);
    
    let worker = MacOsWorker {
        manager: sender,
        module_id,
        is_paused: std::sync::atomic::AtomicBool::new(true),
        player_name: None,
        image_store,
        image_id,
        current_track_id: Arc::new(Mutex::new(None)),
        last_log_time: Arc::new(Mutex::new(Instant::now() - time::Duration::from_secs(30))),
    };
    
    // Crear un canal para recibir actualizaciones de estado
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    
    // Iniciar el listener con los reproductores configurados
    let listener = player::PlayerListener::new(config.players, 1000);
    let _listener_handle = listener.listen(tx);
    
    // Crear la tarea para procesar los estados recibidos
    tokio::spawn(async move {
        info!("Esperando eventos de reproductores de macOS");
        
        while let Some(state) = rx.recv().await {
            worker.feed_manager(state).await;
        }
        
        info!("Módulo macOS finalizado");
    })
}

impl MacOsWorker {
    async fn feed_manager(&self, state: State) {
        // Verificar si hay una canción válida
        let has_title = !state.title.is_empty();
        let has_artist = !state.artist.is_empty();
        
        if has_title && has_artist {
            debug!("Canción detectada: {} - {}", state.title, state.artist);
            
            // Actualizar el estado de pausa
            self.is_paused.store(false, std::sync::atomic::Ordering::SeqCst);
            
            // Crear el estado del módulo
            let module_state = self.make_state(state);
            
            // Enviar el estado al gestor
            self.manager.do_send(UpdateModule {
                id: self.module_id,
                state: module_state,
            });
        } else {
            // No hay música reproduciéndose
            let was_playing = !self.is_paused.swap(true, std::sync::atomic::Ordering::SeqCst);
            
            if was_playing {
                debug!("No hay música reproduciéndose");
                
                // Enviar estado de pausa
                self.manager.do_send(UpdateModule {
                    id: self.module_id,
                    state: ModuleState::Paused,
                });
            }
        }
    }
    
    fn make_state(&self, state: player::State) -> ModuleState {
        // Verificar si es momento de imprimir el log periódico (cada 30 segundos)
        self.check_periodic_log(&state);
        
        let play_info = PlayInfo {
            title: state.title.clone(),
            artist: state.artist.clone(),
            source: format!("macos-{}", state.player_name.to_lowercase()),
            album: None,
            track_number: None,
            timeline: None,
            image: None,
        };
        
        let mut play_info = if let Some(album) = state.album {
            PlayInfo {
                album: Some(AlbumInfo {
                    title: album.title,
                    track_count: album.track_count,
                }),
                ..play_info
            }
        } else {
            play_info
        };
        
        if let Some(track_number) = state.track_number {
            play_info.track_number = Some(track_number);
        }
        
        if let Some(timeline) = state.timeline {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
                
            play_info.timeline = Some(TimelineInfo {
                progress_ms: timeline.progress.as_millis() as u64,
                duration_ms: timeline.duration.as_millis() as u64,
                ts: now,
                rate: 1.0,
            });
        }
        
        // Crear un ID único para la pista actual para seguimiento de cambios
        let track_id = format!("{}-{}-{}", 
            state.player_name, 
            state.title, 
            state.artist);
        
        // Verificar si es una nueva canción usando el Mutex
        let is_new_track = {
            let last_track_id = self.current_track_id.lock().unwrap();
            match &*last_track_id {
                Some(last_id) => *last_id != track_id,
                None => true,
            }
        };
            
        // Actualizar el track ID usando el Mutex
        if is_new_track {
            let mut last_track_id = self.current_track_id.lock().unwrap();
            if let Some(current_track_id) = &*last_track_id {
                info!("Cambio de canción detectado: {} -> {}", current_track_id, track_id);
            }
            *last_track_id = Some(track_id);
        }
        
        // Usar la imagen de portada si está disponible
        if let Some(artwork_data) = state.artwork_data {
            if !artwork_data.is_empty() {
                // Verificar que los datos de la imagen sean válidos 
                // (al menos tienen el encabezado JPEG básico)
                let is_valid_image = artwork_data.len() > 10 && 
                    (
                        // JPEG header
                        (artwork_data[0] == 0xFF && artwork_data[1] == 0xD8) ||
                        // PNG header
                        (artwork_data[0] == 0x89 && artwork_data[1] == 0x50 && 
                         artwork_data[2] == 0x4E && artwork_data[3] == 0x47)
                    );
                
                // Reducir logs excesivos - solo mostrar primeros bytes en debug
                if debug_enabled() {
                    let hex_dump = artwork_data.iter().take(8)
                        .map(|b| format!("{:02X}", b))
                        .collect::<Vec<_>>()
                        .join(" ");
                    
                    debug!("Datos de imagen recibidos: {} bytes. Primeros bytes: {}", 
                         artwork_data.len(), hex_dump);
                }
                
                if !is_valid_image {
                    warn!("Los datos recibidos no parecen ser una imagen válida ({} bytes)", 
                         artwork_data.len());
                    
                    // Retornar sin la imagen
                    return ModuleState::Playing(play_info);
                }
                
                // Determinar el tipo de contenido basado en los datos
                let content_type = if artwork_data[0] == 0xFF && artwork_data[1] == 0xD8 {
                    debug!("Formato JPEG detectado");
                    "image/jpeg".to_string()
                } else if artwork_data[0] == 0x89 && artwork_data[1] == 0x50 && 
                          artwork_data[2] == 0x4E && artwork_data[3] == 0x47 {
                    debug!("Formato PNG detectado");
                    "image/png".to_string()
                } else {
                    warn!("Formato no reconocido, asumiendo JPEG");
                    "image/jpeg".to_string() // Por defecto asumimos JPEG
                };
                
                // SOLUCIÓN: Siempre almacenar una nueva imagen si es una nueva canción para evitar el problema de caché
                let epoch = if is_new_track {
                    // Para una nueva canción, siempre almacenamos una nueva imagen
                    if let Ok(mut store) = self.image_store.write() {
                        debug!("Almacenando nueva imagen para nueva canción");
                        let epoch = store.store(*self.image_id, content_type, artwork_data);
                        debug!("Nueva portada almacenada (id={}, epoch={})", *self.image_id, epoch);
                        epoch
                    } else {
                        warn!("No se pudo escribir en el almacén de imágenes");
                        0 // Valor por defecto si hay error
                    }
                } else {
                    // Verificar si ya tenemos esta imagen almacenada
                    if let Ok(store) = self.image_store.read() {
                        if let Some((epoch, _)) = store.get_latest(*self.image_id) {
                            // Reutilizar la imagen existente
                            debug!("Reutilizando portada existente (id={}, epoch={})", *self.image_id, epoch);
                            epoch
                        } else {
                            // No hay imagen previa
                            if let Ok(mut store) = self.image_store.write() {
                                let epoch = store.store(*self.image_id, content_type, artwork_data);
                                debug!("Almacenando primera imagen (id={}, epoch={})", *self.image_id, epoch);
                                epoch
                            } else {
                                0
                            }
                        }
                    } else {
                        warn!("No se pudo leer el almacén de imágenes");
                        0
                    }
                };
                
                // Asignar la imagen a PlayInfo
                play_info.image = Some(ImageInfo::Internal(InternalImage {
                    id: *self.image_id,
                    epoch_id: epoch,
                }));
                
                debug!("Portada disponible para {} - {} (id={}, epoch={})", 
                     play_info.title, play_info.artist, *self.image_id, epoch);
            }
        }
        
        ModuleState::Playing(play_info)
    }
    
    // Función para imprimir un log periódico con el formato solicitado
    fn check_periodic_log(&self, state: &State) {
        let now = Instant::now();
        let should_log = {
            let mut last_log = self.last_log_time.lock().unwrap();
            let elapsed = now.duration_since(*last_log);
            
            if elapsed >= time::Duration::from_secs(30) {
                *last_log = now;
                true
            } else {
                false
            }
        };
        
        if should_log {
            let current_time = state.timeline.as_ref().map_or("00:00".to_string(), |t| 
                format!("{:02}:{:02}", 
                    t.progress.as_secs() / 60, 
                    t.progress.as_secs() % 60
                )
            );
            
            let duration = state.timeline.as_ref().map_or("00:00".to_string(), |t| 
                format!("{:02}:{:02}", 
                    t.duration.as_secs() / 60, 
                    t.duration.as_secs() % 60
                )
            );
            
            let has_image = state.artwork_data.as_ref().map_or(false, |data| !data.is_empty());
            
            // Formato: [song title] - [artist name] - [source provider] - ([currenttime]/[duration]) - [is image valid]
            info!(
                "{} - {} - {} - ({}/{}) - imagen: {}", 
                state.title, 
                state.artist, 
                state.player_name, 
                current_time,
                duration,
                if has_image { "sí" } else { "no" }
            );
        }
    }
}

// Función auxiliar para verificar si los logs de nivel debug están habilitados
#[inline]
fn debug_enabled() -> bool {
    cfg!(debug_assertions)
} 