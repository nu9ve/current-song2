import { formatLocalUrl } from '../../shared/url';
import { PlayInfo } from '../../shared/types';

// URL para imagen de respaldo en caso de error
const FALLBACK_IMAGE_URL = '/img/default-cover.jpg'; 

export function getImageUrl(info: PlayInfo): string | undefined {
  if (!info.image) {
    console.log('Sin imagen disponible para:', info.title, info.artist);
    return undefined;
  }

  try {
    if (typeof info.image === 'string') {
      if (!info.image.startsWith('https://')) {
        console.warn('URL de imagen externa inválida:', info.image);
        return undefined;
      }
      // console.log('Imagen externa:', info.image);
      return info.image;
    } else {
      const imageUrl = formatLocalUrl({
        path: `/api/img/${info.image.id}/${info.image.epochId}`,
        port: Number(location.port) || 48457,
        host: location.hostname,
      });
      // console.log('Imagen interna:', imageUrl, 'ID:', info.image.id, 'Epoch:', info.image.epochId);
      return imageUrl;
    }
  } catch (error) {
    console.error('Error generando URL de imagen:', error);
    // En caso de error con la imagen, podemos devolver una imagen de respaldo
    // return FALLBACK_IMAGE_URL;
    return undefined;
  }
}
