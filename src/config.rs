#![deny(unused_must_use)]

use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tap::TapFallible;
use tracing::warn;

macro_rules! cfg_unix {
    ($($tokens:tt)*) => {
        #[cfg(unix)]
        $($tokens)*
    };
}

macro_rules! cfg_windows {
    ($($tokens:tt)*) => {
        #[cfg(windows)]
        $($tokens)*
    };
}

macro_rules! cfg_macos {
    ($($tokens:tt)*) => {
        #[cfg(target_os = "macos")]
        $($tokens)*
    };
}

/// Configuración del servidor
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum BindConfig {
    /// Configuración sencilla para hacer bind en localhost:<port>
    Single {
        port: u16,
    },
    /// Bind en múltiples IPs
    Multiple {
        bind: Vec<(String, u16)>,
    },
}

impl Default for BindConfig {
    fn default() -> Self {
        Self::Single { port: 48457 }
    }
}

/// Configuración del servidor web
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(default)]
pub struct ServerConfig {
    #[serde(flatten)]
    pub bind: BindConfig,
    pub custom_theme_path: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: BindConfig::default(),
            custom_theme_path: "theme.css".to_owned(),
        }
    }
}

/// Configuración de salida a archivo
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct FileOutputFormat {
    #[serde(default = "std::string::String::new")]
    pub when_playing: String,
    #[serde(default = "std::string::String::new")]
    pub when_paused: String,
}

impl Default for FileOutputFormat {
    fn default() -> Self {
        Self {
            when_playing: "{title} - {artist}".to_owned(),
            when_paused: "".to_owned(),
        }
    }
}

/// Configuración de la salida del archivo
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(default)]
pub struct FileOutputConfig {
    #[serde(default = "bool_false")]
    pub enabled: bool,
    pub path: String,
    pub format: FileOutputFormat,
}

impl Default for FileOutputConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: "current_song.txt".to_owned(),
            format: FileOutputFormat::default(),
        }
    }
}

/// Configuración de filtrado GSMTC
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(default)]
pub struct FilterConfig {
    pub mode: FilterMode,
    pub items: Vec<String>,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            // Predeterminadamente, excluir Chrome, Edge y Firefox, ya que es probable
            // que solo aparezcan videos de YouTube y no es fácil determinar los anuncios.
            mode: FilterMode::Exclude,
            items: vec![
                "chrome.exe".to_owned(),
                "msedge.exe".to_owned(),
                "firefox.exe".to_owned(),
            ],
        }
    }
}

/// Modo de filtrado
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum FilterMode {
    /// No excluir ni incluir nada, aceptar todo
    Disabled,
    /// Lista de aplicaciones a incluir (rechazar el resto)
    Include,
    /// Lista de aplicaciones a excluir (aceptar el resto)
    Exclude,
}

cfg_windows! {
    /// Configuración para GSMTC (Windows)
    #[derive(Deserialize, Serialize, Debug, Clone, Default)]
    #[serde(default)]
    pub struct GsmtcConfig {
        #[serde(default = "bool_true")]
        pub enabled: bool,
        pub filter: FilterConfig,
    }
}

cfg_unix! {
    #[derive(Deserialize, Serialize, Debug, Clone)]
    #[serde(default)]
    pub struct DbusConfig {
        #[serde(default = "bool_true")]
        pub enabled: bool,
        pub destinations: Vec<String>,
    }

    impl Default for DbusConfig {
        fn default() -> Self {
            Self {
                enabled: true,
                destinations: vec!["org.mpris.MediaPlayer2.*".to_owned()],
            }
        }
    }
}

cfg_macos! {
    #[derive(Deserialize, Serialize, Debug, Clone)]
    #[serde(default)]
    pub struct MacOsConfig {
        #[serde(default = "bool_true")]
        pub enabled: bool,
        pub players: Vec<String>,
    }

    impl Default for MacOsConfig {
        fn default() -> Self {
            Self {
                enabled: true,
                players: vec!["Spotify".to_owned(), "Music".to_owned()],
            }
        }
    }
}

/// Configuración de los módulos
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(default)]
pub struct ModuleConfig {
    #[cfg(windows)]
    pub gsmtc: GsmtcConfig,
    #[cfg(unix)]
    pub dbus: DbusConfig,
    #[cfg(target_os = "macos")]
    pub macos: MacOsConfig,
    pub file: FileOutputConfig,
}

impl Default for ModuleConfig {
    fn default() -> Self {
        Self {
            #[cfg(windows)]
            gsmtc: GsmtcConfig::default(),
            #[cfg(unix)]
            dbus: DbusConfig::default(),
            #[cfg(target_os = "macos")]
            macos: MacOsConfig::default(),
            file: FileOutputConfig::default(),
        }
    }
}

/// Configuración general
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(default)]
pub struct Config {
    #[serde(default = "bool_true")]
    pub no_autostart: bool,

    pub modules: ModuleConfig,
    pub server: ServerConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            no_autostart: true,

            modules: ModuleConfig::default(),
            server: ServerConfig::default(),
        }
    }
}

fn bool_true() -> bool {
    true
}

fn bool_false() -> bool {
    false
}

static CURRENT_CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();

lazy_static::lazy_static! {
    pub static ref CONFIG: Config = {
        loop {
            match read_config() {
                Ok((path, config)) => {
                    CURRENT_CONFIG_PATH.get_or_init(|| path);
                    break config
                },
                Err(None) => {
                    warn!("Didn't find any config at any location, creating default one at default location");
                    let conf = Config::default();

                    let path = default_config_paths()[0].clone(); // can't move out of array
                    save_config(&conf, &path).ok();
                    CURRENT_CONFIG_PATH.get_or_init(|| path);

                    break conf
                }
                Err(Some((loc, err))) => {
                    #[cfg(windows)]
                    if crate::win_setup::should_replace_invalid_config(&loc, &err) {
                        #[allow(clippy::redundant_clone)]
                        let conf = Config::default();
                        save_config(&conf, &loc).ok();
                        CURRENT_CONFIG_PATH.get_or_init(|| loc);
                        break conf
                    } else {
                        continue;
                    }

                    #[cfg(not(windows))]
                    {
                        tracing::error!("Failed to load config at {} ({}); using defaults", loc.display(), err);
                        let conf = Config::default();
                        CURRENT_CONFIG_PATH.get_or_init(|| loc);
                        break conf
                    }
                }
            }
        }
    };
}

pub fn current_config_path() -> &'static Path {
    CURRENT_CONFIG_PATH.get().expect("Path should've been initialized")
}

fn default_config_paths() -> [PathBuf; 2] {
    match default_config_dir() {
        Some(mut p1) => {
            p1.push("config.toml");
            [p1, Path::new("config.toml").to_owned()]
        }
        None => [Path::new("config.toml").to_owned(), Path::new("config.toml").to_owned()],
    }
}

fn default_config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        match std::env::var_os("APPDATA") {
            Some(app_data) => {
                let mut p = PathBuf::from(app_data);
                p.push("CurrentSong2");
                std::fs::create_dir_all(&p).tap_err(|e| {
                    warn!("Failed to create config directory {}: {}", p.display(), e);
                }).ok()?;
                Some(p)
            }
            None => {
                warn!("'APPDATA' environment variable not set");
                None
            }
        }
    }

    #[cfg(not(windows))]
    {
        match dirs::config_dir() {
            Some(mut p) => {
                p.push("CurrentSong2");
                std::fs::create_dir_all(&p).tap_err(|e| {
                    warn!("Failed to create config directory {}: {}", p.display(), e);
                }).ok()?;
                Some(p)
            }
            None => {
                warn!("Failed to determine config directory");
                None
            }
        }
    }
}

fn read_config() -> Result<(PathBuf, Config), Option<(PathBuf, &'static str)>> {
    let mut last_loc = None;
    for path in default_config_paths() {
        match File::open(&path) {
            Ok(file) => {
                let mut content = String::new();
                if let Err(_) = std::io::Read::read_to_string(&mut BufReader::new(file), &mut content) {
                    return Err(Some((path, "Couldn't read config file")));
                }
                
                return match toml::from_str(&content) {
                    Ok(config) => Ok((path, config)),
                    Err(_) => Err(Some((path, "Couldn't parse config"))),
                };
            }
            Err(err) => {
                last_loc = if err.kind() == std::io::ErrorKind::NotFound {
                    last_loc
                } else {
                    Some(path)
                };
            }
        }
    }

    if let Some(loc) = last_loc {
        Err(Some((loc, "File exists but couldn't be opened")))
    } else {
        Err(None)
    }
}

pub fn save_config(config: &Config, path: impl AsRef<Path>) -> std::io::Result<()> {
    let output = toml::to_string_pretty(config).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to serialize config: {e}"),
        )
    })?;

    let mut file = File::create(path)?;
    file.write_all(output.as_bytes())?;

    Ok(())
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(default)]
pub struct MprisConfig {
    #[serde(default = "bool_true")]
    pub enabled: bool,
    pub destinations: Vec<String>,
}

impl Default for MprisConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            destinations: vec!["org.mpris.MediaPlayer2.*".to_owned()],
        }
    }
}
