use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use actix::Addr;
use macos_media::player::{self, State};
use tokio::task::JoinHandle;
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
                
                // Imprimir los primeros bytes para diagnóstico
                let hex_dump = artwork_data.iter().take(32)
                    .map(|b| format!("{:02X}", b))
                    .collect::<Vec<_>>()
                    .join(" ");
                
                info!("Datos de imagen recibidos para {} - {}: {} bytes. Primeros bytes: {}", 
                     play_info.title, play_info.artist, artwork_data.len(), hex_dump);
                
                if !is_valid_image {
                    warn!("Los datos recibidos no parecen ser una imagen válida ({} bytes). Primeros bytes: {}", 
                         artwork_data.len(), hex_dump);
                    
                    // Retornar sin la imagen
                    return ModuleState::Playing(play_info);
                }
                
                // Determinar el tipo de contenido basado en los datos
                let content_type = if artwork_data[0] == 0xFF && artwork_data[1] == 0xD8 {
                    info!("Formato JPEG detectado para la imagen");
                    "image/jpeg".to_string()
                } else if artwork_data[0] == 0x89 && artwork_data[1] == 0x50 && 
                          artwork_data[2] == 0x4E && artwork_data[3] == 0x47 {
                    info!("Formato PNG detectado para la imagen");
                    "image/png".to_string()
                } else {
                    warn!("Formato no reconocido, asumiendo JPEG");
                    "image/jpeg".to_string() // Por defecto asumimos JPEG
                };
                
                // Verificamos si ya tenemos esta imagen almacenada para evitar duplicados
                let (should_store, current_epoch) = {
                    if let Ok(store) = self.image_store.read() {
                        if let Some((epoch, img)) = store.get_latest(*self.image_id) {
                            // Si ya existe una imagen, solo la reemplazamos si:
                            // 1. Es una nueva canción
                            // 2. O si los datos son diferentes (verificando longitud)
                            let size_matches = img.data.len() == artwork_data.len();
                            (!size_matches || is_new_track, Some(epoch))
                        } else {
                            // No hay imagen previa
                            (true, None)
                        }
                    } else {
                        // Error leyendo el almacén, almacenamos por seguridad
                        (true, None)
                    }
                };
                
                // Almacenar imagen y obtener época
                let epoch = if should_store {
                    let epoch = {
                        if let Ok(mut store) = self.image_store.write() {
                            info!("Almacenando nueva imagen de tipo {} ({} bytes)", 
                                 content_type, artwork_data.len());
                            let epoch = store.store(*self.image_id, content_type, artwork_data);
                            info!("Nueva portada almacenada para {} - {} (id={}, epoch={})", 
                                 play_info.title, play_info.artist, *self.image_id, epoch);
                            epoch
                        } else {
                            // Si no podemos escribir en el almacén, usamos la época actual o 0
                            current_epoch.unwrap_or(0)
                        }
                    };
                    epoch
                } else if let Some(epoch) = current_epoch {
                    // Reutilizamos la época existente
                    debug!("Reutilizando portada existente para {} - {} (id={}, epoch={})", 
                           play_info.title, play_info.artist, *self.image_id, epoch);
                    epoch
                } else {
                    0
                };
                
                // Añadimos la referencia a la imagen
                play_info.image = Some(ImageInfo::Internal(InternalImage {
                    id: *self.image_id,
                    epoch_id: epoch,
                }));
                
                info!("Portada disponible para {} - {} (id={}, epoch={})", 
                     play_info.title, play_info.artist, *self.image_id, epoch);
            } else {
                debug!("Portada vacía para {} - {}", play_info.title, play_info.artist);
            }
        } else {
            debug!("Sin portada para {} - {}", play_info.title, play_info.artist);
        }
        
        ModuleState::Playing(play_info)
    }
} 