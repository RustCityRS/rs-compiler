use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub install_dir: PathBuf,
    pub scripts_dir: PathBuf,
    pub env_name: String,
}

impl Default for Config {
    fn default() -> Self {
        let env_name = env::var("RSC_ENV").unwrap_or_else(|_| String::from("default"));
        let base_dir = if cfg!(windows) {
            PathBuf::from(env::var("USERPROFILE").unwrap_or_else(|_| String::from(".")))
        } else {
            PathBuf::from(env::var("HOME").unwrap_or_else(|_| String::from(".")))
        };

        let install_dir = if let Ok(custom_dir) = env::var("RSC_INSTALL_DIR") {
            PathBuf::from(custom_dir)
        } else {
            base_dir.join(".rsc").join(&env_name)
        };

        let scripts_dir = if let Ok(custom_dir) = env::var("RSC_SCRIPTS_DIR") {
            PathBuf::from(custom_dir)
        } else {
            // First check if there's a local scripts directory
            let local_scripts = Path::new("./data/scripts");
            if local_scripts.is_dir() {
                local_scripts.to_path_buf()
            } else {
                install_dir.join("scripts")
            }
        };

        Config {
            install_dir,
            scripts_dir,
            env_name,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let config_path = Self::get_config_path();
        let mut config = if config_path.exists() {
            fs::read_to_string(&config_path)
                .ok()
                .and_then(|c| serde_json::from_str(&c).ok())
                .unwrap_or_default()
        } else {
            let config = Config::default();
            config.save().unwrap_or_default();
            config
        };

        // Env vars always override the saved config
        if let Ok(dir) = env::var("RSC_SCRIPTS_DIR") {
            config.scripts_dir = PathBuf::from(dir);
        }
        if let Ok(dir) = env::var("RSC_INSTALL_DIR") {
            config.install_dir = PathBuf::from(dir);
        }
        if let Ok(name) = env::var("RSC_ENV") {
            config.env_name = name;
        }

        config
    }

    pub fn save(&self) -> io::Result<()> {
        let config_path = Self::get_config_path();
        fs::create_dir_all(config_path.parent().unwrap())?;

        let contents = serde_json::to_string_pretty(self)?;
        fs::write(&config_path, contents)
    }

    pub fn get_config_path() -> PathBuf {
        let env_name = env::var("RSC_ENV").unwrap_or_else(|_| String::from("default"));
        if cfg!(windows) {
            PathBuf::from(env::var("USERPROFILE").unwrap_or_else(|_| String::from(".")))
                .join(".rsc")
                .join(&env_name)
                .join("config.json")
        } else {
            PathBuf::from(env::var("HOME").unwrap_or_else(|_| String::from(".")))
                .join(".rsc")
                .join(&env_name)
                .join("config.json")
        }
    }

    pub fn get_binary_name() -> &'static str {
        if cfg!(windows) { "rsc.exe" } else { "rsc" }
    }

    pub fn get_binary_path(&self) -> PathBuf {
        self.install_dir.join("bin").join(Self::get_binary_name())
    }
}
