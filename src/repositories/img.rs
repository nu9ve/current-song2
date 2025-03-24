use crate::image_store::ImageStore;
use actix_web::{error, get, web, HttpResponse, Result};
use std::sync::RwLock;
use tracing::{debug, info, warn};

#[get("/{id}/{target_epoch}")]
async fn get_image(
    path: web::Path<(usize, usize)>,
    store: web::Data<RwLock<ImageStore>>,
) -> Result<HttpResponse> {
    let (id, target_epoch) = path.into_inner();
    let image_store = store.into_inner();
    
    // Intentamos obtener la imagen con el bloqueo mínimo
    let img_result = {
        let store = image_store.read().map_err(|e| {
            warn!("Error al bloquear ImageStore para lectura: {}", e);
            error::ErrorInternalServerError("Error al acceder al almacén de imágenes")
        })?;
        
        // Intentar obtener la imagen con la época específica
        match store.get(id, target_epoch) {
            Some(img) => Ok((img.content_type.clone(), img.data.clone())),
            None => {
                // Si la imagen específica no existe, intentar obtener la última época
                match store.get_latest(id) {
                    Some((latest_epoch, img)) => {
                        debug!("Imagen solicitada id={} epoch={} no encontrada, sirviendo la más reciente epoch={}", 
                            id, target_epoch, latest_epoch);
                        Ok((img.content_type.clone(), img.data.clone()))
                    },
                    None => {
                        debug!("Imagen no encontrada id={} epoch={}", id, target_epoch);
                        Err(error::ErrorNotFound(format!("Imagen id={} epoch={} no existe", id, target_epoch)))
                    }
                }
            }
        }
    };
    
    // Procesamos el resultado
    match img_result {
        Ok((content_type, data)) => {
            // Verificamos que los datos parezcan una imagen válida
            let valid_image = data.len() > 4 && (
                // JPEG header
                (data[0] == 0xFF && data[1] == 0xD8) ||
                // PNG header
                (data[0] == 0x89 && data[1] == 0x50 && data[2] == 0x4E && data[3] == 0x47)
            );
            
            if !valid_image {
                let hex_dump = data.iter()
                    .take(32)
                    .map(|b| format!("{:02X}", b))
                    .collect::<Vec<_>>()
                    .join(" ");
                
                warn!("Imagen id={} epoch={} no tiene formato válido. Longitud={} bytes, primeros bytes hex: {}",
                     id, target_epoch, data.len(), hex_dump);
                return Err(error::ErrorInternalServerError("Datos de imagen inválidos"));
            }
            
            info!("Sirviendo imagen id={} epoch={} tipo={} tamaño={} bytes, primeros bytes: {:02X} {:02X} {:02X} {:02X}", 
                 id, target_epoch, content_type, data.len(), 
                 data[0], data[1], 
                 data.get(2).copied().unwrap_or(0), 
                 data.get(3).copied().unwrap_or(0));
            
            // Creamos los encabezados como cadenas primero
            let size_header = format!("{}", data.len());
            let id_header = format!("{}", id);
            let epoch_header = format!("{}", target_epoch);
            let headers_header = format!("{:02X} {:02X} {:02X} {:02X}", 
                data[0], data[1], 
                data.get(2).copied().unwrap_or(0), 
                data.get(3).copied().unwrap_or(0));
                
            // Usamos un enfoque más simple con el builder
            let mut builder = HttpResponse::Ok();
            builder.content_type(content_type.clone());
            builder.append_header(("X-Image-Size", size_header));
            builder.append_header(("X-Image-ID", id_header));
            builder.append_header(("X-Image-Epoch", epoch_header));
            builder.append_header(("X-Image-Headers", headers_header));
                
            if content_type == "image/jpeg" {
                builder.append_header(("Cache-Control", "public, max-age=3600"));
            }
            
            Ok(builder.body(data))
        },
        Err(e) => Err(e)
    }
}

pub fn init_img(config: &mut web::ServiceConfig) {
    config.service(get_image);
}
