import { cleanupTitleAndSub, extractTitleAndSub } from './text';
import { getImageUrl } from './image';
import { PlayInfo } from '../../shared/types';
import { ScriptOptions } from './options';

export interface State {
  info: PlayInfo;
  title: string;
  subtitle: string | undefined;
  imageUrl: string | undefined;
}

export function makeState(info: PlayInfo, options: ScriptOptions): State {
  let extracted = extractTitleAndSub(info);
  if (!options.useRawSongInfo) {
    extracted = cleanupTitleAndSub(extracted);
  }
  const { title, subtitle } = extracted;

  const imageUrl = getImageUrl(info);
  return {
    info,
    title,
    subtitle,
    imageUrl,
  };
}

export function isSpotify(state: State): boolean {
  return (
    state.info.source.toLowerCase().includes('spotify')
  );
}

export function isAppleMusic(state: State): boolean {
  return (
    state.info.source.toLowerCase().includes('music')
  );
}

export function getProviderName(state: State): string {
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

export function hasImage(state: State) {
  return !!state.imageUrl;
}

export function hasSubtitle(state: State) {
  return !!state.subtitle;
}

export function hasTimeline(state: State) {
  return !!state.info.timeline;
}

export function hasAlbum(state: State) {
  return !!state.info.album;
}

export function hasValidAlbumTracks(state: State) {
  return (state.info.album?.trackCount ?? 0) > 0;
}

export function hasTrack(state: State) {
  return (state.info.trackNumber ?? 0) > 0;
}

export function not<T>(fn: (state: T) => boolean): (state: T) => boolean {
  return s => !fn(s);
}
