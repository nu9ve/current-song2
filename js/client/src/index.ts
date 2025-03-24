import { getElements } from './dom/utils';
import { createProgress } from './progress';
import { smolTree } from './dom/smol-tree';
import {
  hasAlbum,
  hasImage,
  hasSubtitle,
  hasTimeline,
  hasTrack,
  hasValidAlbumTracks,
  isSpotify,
  isAppleMusic,
  makeState,
  not,
  State,
} from './state';
import { animateOnChange, TextChangeAnimation } from './dom/animation';
import { EventMap } from './types';
import { formatLocalUrl } from '../../shared/url';
import {
  IncomingMessages,
  OutgoingMessages,
  ReconnectingWebsocket,
} from '../../shared/reconnecting-websocket';
import { startUserScript } from './user-scripts';
import { MarqueeEl, MarqueeOptions, wrapMarquee } from './text/marquee';
import { setupOptions } from './options';

// Función para obtener el nombre del proveedor
function getProviderName(state: State): string {
  const source = state.info.source.toLowerCase();
  console.log("Source original:", state.info.source);
  
  // Para depuración, mostrar la fuente original
  console.log("Analizando fuente:", source);
  
  // Caso específico para Music en macOS (sin prefijo)
  if (source === 'music') {
    console.log("Detectado Apple Music directamente");
    return 'Apple Music';
  }
  
  // Manejar específicamente las fuentes de macOS
  if (source.startsWith('macos-')) {
    const macPlayer = source.substring(6); // Quitar el prefijo 'macos-'
    console.log("Reproductor macOS detectado:", macPlayer);
    
    if (macPlayer === 'spotify') {
      return 'Spotify';
    } else if (macPlayer === 'music') {
      return 'Apple Music';
    } else if (macPlayer === 'itunes') {
      return 'iTunes';
    } else {
      // Capitalizar el nombre del reproductor
      return macPlayer.charAt(0).toUpperCase() + macPlayer.slice(1);
    }
  }
  
  // Procesamiento normal para otras fuentes
  if (source.includes('spotify')) {
    return 'Spotify';
  } else if (source.includes('music') || source.includes('apple')) {
    return 'Apple Music';
  } else if (source.includes('itunes')) {
    return 'iTunes';
  } else if (source.includes('chrome')) {
    return 'Chrome';
  } else if (source.includes('firefox')) {
    return 'Firefox';
  } else if (source.includes('edge')) {
    return 'Edge';
  } else if (source.includes('vlc')) {
    return 'VLC';
  } else if (source.includes('soundcloud')) {
    return 'SoundCloud';
  } else if (source.includes('youtube')) {
    return 'YouTube';
  } else if (source.includes('tidal')) {
    return 'Tidal';
  } else if (source.includes('deezer')) {
    return 'Deezer';
  } else if (source.includes('pandora')) {
    return 'Pandora';
  } else if (source.includes('amazon') || source.includes('prime')) {
    return 'Amazon Music';
  } else if (source.includes('audible')) {
    return 'Audible';
  } else {
    // Imprimir la fuente para depuración
    console.log("Fuente desconocida:", source);
    
    // Extraer nombre del reproductor de la fuente
    const parts = source.split('-');
    if (parts.length > 1) {
      // Capitalizar el nombre
      return parts[1].charAt(0).toUpperCase() + parts[1].slice(1);
    }
    
    // Si no podemos determinar nada, usar el string completo capitalizado
    if (source && source.length > 0) {
      return source.charAt(0).toUpperCase() + source.slice(1);
    }
    
    return 'Reproductor';
  }
}

function wrapMarqueeElements(
  root: HTMLElement,
  titleEl: HTMLElement,
  subtitleEl: HTMLElement,
): MarqueeEl {
  const style = getComputedStyle(root);
  const useIt = style.getPropertyValue('--use-marquee').trim() === 'true';
  if (!useIt) {
    return { pause() {}, reset() {}, start() {} };
  }
  const opt = (name: string, defaultValue: number) => {
    const parsed = parseFloat(style.getPropertyValue(name).trim());
    return Number.isNaN(parsed) ? defaultValue : parsed;
  };
  const opts: MarqueeOptions = {
    speed: opt('--marquee-speed', 0.2),
    pauseDuration: opt('--marquee-pause-duration', 1200),
    repeatPauseDuration: opt('--marquee-repeat-pause-duration', 2000),
  };
  const title = wrapMarquee(titleEl, opts);
  const subtitle = wrapMarquee(subtitleEl, opts);

  return {
    pause: () => {
      title.pause();
      subtitle.pause();
    },
    start: () => {
      title.start();
      subtitle.start();
    },
    reset: () => {
      title.reset();
      subtitle.reset();
    },
  };
}

// Función para cargar imágenes con retry
function loadImageWithRetry(
  imageEl: HTMLImageElement,
  imageUrl: string, 
  maxRetries = 3, 
  retryDelay = 1000
): Promise<boolean> {
  return new Promise((resolve) => {
    let retries = 0;
    
    const tryLoad = () => {
      if (retries >= maxRetries) {
        console.error(`No se pudo cargar la imagen después de ${maxRetries} intentos:`, imageUrl);
        resolve(false);
        return;
      }
      
      const tempImage = new Image();
      
      tempImage.onload = () => {
        // La imagen cargó correctamente, actualizar la imagen real
        imageEl.src = imageUrl;
        resolve(true);
      };
      
      tempImage.onerror = () => {
        retries++;
        console.warn(`Error al cargar imagen (intento ${retries}/${maxRetries}):`, imageUrl);
        
        if (retries < maxRetries) {
          setTimeout(tryLoad, retryDelay);
        } else {
          resolve(false);
        }
      };
      
      // Intentar cargar la imagen
      tempImage.src = imageUrl;
    };
    
    tryLoad();
  });
}

(async function main() {
  // Detectar y aplicar el tema de la URL
  function applyThemeFromUrl() {
    const urlParams = new URLSearchParams(window.location.search);
    const theme = urlParams.get('theme');
    
    console.log('Aplicando tema. Parámetro URL:', theme);
    
    if (theme === 'computer') {
      document.documentElement.setAttribute('data-theme', 'computer');
      console.log('Tema computer aplicado. Verificando:', document.documentElement.getAttribute('data-theme'));
      
      // Forzar la aplicación de estilos
      document.documentElement.classList.add('theme-computer');
    } else {
      // Tema default o cualquier otro valor desconocido
      document.documentElement.setAttribute('data-theme', 'default');
      console.log('Tema default aplicado. Verificando:', document.documentElement.getAttribute('data-theme'));
      
      // Eliminar la clase si existe
      document.documentElement.classList.remove('theme-computer');
    }
  }
  
  // Aplicar el tema según la URL
  applyThemeFromUrl();
  
  // Actualizar el tema cuando cambie la URL
  window.addEventListener('popstate', applyThemeFromUrl);
  
  const [container, imageContainer, imageEl, titleEl, subtitleEl, progressEl, providerEl] = getElements<
    [
      HTMLDivElement,
      HTMLDivElement,
      HTMLImageElement,
      HTMLHeadingElement,
      HTMLHeadingElement,
      HTMLDivElement,
      HTMLDivElement,
    ]
  >('song-container', 'image-container', 'image', 'title', 'subtitle', 'progress', 'provider');
  const resetMarquee = wrapMarqueeElements(container, titleEl, subtitleEl);

  const progressManager = createProgress(progressEl);

  const tree = smolTree<State>(
    [imageEl, { spotify: isSpotify }],
    [imageContainer, { hidden: not(hasImage) }],
    [
      container,
      {
        'with-image': hasImage,
        'is-spotify': isSpotify,
        'with-progress': hasTimeline,
        'has-album-tracks': hasValidAlbumTracks,
        'has-track': hasTrack,
        'with-album': hasAlbum,
      },
    ],
    [subtitleEl, { hidden: not(hasSubtitle) }],
  );
  const scriptOptions = setupOptions();

  const userScript = startUserScript();

  // Añadir evento de error para la imagen
  imageEl.addEventListener('error', (e) => {
    console.error('Error al cargar la imagen:', imageEl.src, e);
    // Ocultar el contenedor de imagen si falla la carga
    imageContainer.classList.add('hidden');
  });

  const ws = new ReconnectingWebsocket<IncomingMessages<EventMap>, OutgoingMessages>(
    formatLocalUrl({
      path: '/api/ws/client',
      port: Number(location.port) || 48457,
      protocol: 'ws',
      host: location.hostname,
    }),
  );
  ws.addEventListener('Playing', ({ data }) => {
    container.classList.remove('vanish');
    const state = makeState(data, scriptOptions);
    tree.update(state);
    resetMarquee.start();

    // Depuración de datos recibidos del servidor
    console.log("Datos recibidos del servidor:", data);
    console.log("Fuente original:", data.source);

    animateOnChange(titleEl, state.title, resetMarquee.reset, ...TextChangeAnimation);
    if (state.subtitle) {
      animateOnChange(subtitleEl, state.subtitle, resetMarquee.reset, ...TextChangeAnimation);
    }

    // Mostrar el proveedor de música
    if (providerEl) {
      const providerName = getProviderName(state);
      console.log("Provider:", providerName, "Source:", state.info.source); // Debug
      
      // Establecer el contenido del proveedor
      providerEl.textContent = providerName;
      
      // Forzar que el proveedor sea visible siempre
      providerEl.style.display = 'inline-block';
      providerEl.style.visibility = 'visible'; // Asegurar visibilidad
      providerEl.classList.remove('hidden');
      
      // FORZAR UN VALOR SIEMPRE PARA DIAGNÓSTICO
      if (!providerEl.textContent || providerEl.textContent.trim() === '') {
        console.log("DETECCIÓN DE PROVIDER VACÍO - FORZANDO VALOR");
        if (state.info.source === 'Music') {
          providerEl.textContent = 'Apple Music';
          providerEl.classList.add('apple-music');
        } else {
          providerEl.textContent = state.info.source || 'Reproductor';
        }
      }
      
      // Agregar clases para estilos específicos por proveedor
      providerEl.className = 'provider'; // Resetear clases
      
      // Agregar clases específicas basadas en el proveedor
      const source = state.info.source.toLowerCase();
      
      // Caso específico para 'Music' de macOS
      if (source === 'music') {
        providerEl.classList.add('apple-music');
      }
      // Otros casos
      else if (source.includes('spotify') || (source.startsWith('macos-') && source.includes('spotify'))) {
        providerEl.classList.add('spotify');
      } else if (source.includes('music') || source.includes('apple') || 
                (source.startsWith('macos-') && source.includes('music'))) {
        providerEl.classList.add('apple-music');
      } else if (source.includes('soundcloud')) {
        providerEl.classList.add('soundcloud');
      } else if (source.includes('youtube')) {
        providerEl.classList.add('youtube');
      } else if (source.includes('tidal')) {
        providerEl.classList.add('tidal');
      } else if (source.includes('deezer')) {
        providerEl.classList.add('deezer');
      } else if (source.includes('pandora')) {
        providerEl.classList.add('pandora');
      } else if (source.includes('amazon') || source.includes('prime')) {
        providerEl.classList.add('amazon-music');
      } else if (source.includes('audible')) {
        providerEl.classList.add('audible');
      } else if (source.includes('vlc')) {
        providerEl.classList.add('vlc');
      }
    }

    // Mejorar la gestión de imágenes para evitar problemas de caché
    if (state.imageUrl && typeof state.imageUrl === 'string') {
      // Añadir un parámetro timestamp para evitar la caché del navegador
      const timestamp = new Date().getTime();
      const imageUrlWithCache = state.imageUrl.includes('?') 
        ? `${state.imageUrl}&_t=${timestamp}` 
        : `${state.imageUrl}?_t=${timestamp}`;
      
      loadImageWithRetry(imageEl, imageUrlWithCache).then(success => {
        if (success) {
          container.style.setProperty('--image-url', `url("${encodeURI(imageUrlWithCache)}")`);
          imageContainer.classList.remove('hidden');
        } else {
          console.warn('No se pudo cargar la imagen, ocultando el contenedor');
          imageContainer.classList.add('hidden');
        }
      });
    } else {
      // Si no hay imagen, ocultar el contenedor
      imageContainer.classList.add('hidden');
    }
    
    container.style.setProperty('--title', JSON.stringify(data.title));
    container.style.setProperty('--artist', JSON.stringify(data.artist));
    if (data.album) {
      container.style.setProperty('--album', JSON.stringify(data.album.title));
      container.style.setProperty(
        '--album-tracks',
        JSON.stringify(data.album.trackCount.toString()),
      );
    }
    if (data.trackNumber) {
      container.style.setProperty('--track-number', JSON.stringify(data.trackNumber.toString()));
    }

    progressManager.run(data.timeline);

    userScript.onPlay(state);
  });
  ws.addEventListener('Paused', () => {
    container.classList.add('vanish');
    progressManager.pause();

    userScript.onPause();
    resetMarquee.pause();
  });
  await ws.connect();
})();
