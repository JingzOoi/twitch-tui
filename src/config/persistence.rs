use std::{
    env,
    fs::{File, OpenOptions},
    io::Write,
    mem::drop,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use color_eyre::Result;
use directories::ProjectDirs;
use tokio::{runtime::Handle, task};

use crate::config::CoreConfig;

static PACKAGE_NAME: LazyLock<String> = LazyLock::new(|| env!("CARGO_PKG_NAME").to_lowercase());
static BINARY_NAME: LazyLock<String> = LazyLock::new(|| env!("CARGO_BIN_NAME").to_lowercase());
static CONFIG_DIR: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    env::var(format!("{}_CONFIG", BINARY_NAME.clone()))
        .ok()
        .map(PathBuf::from)
});
static CACHE_DIR: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    env::var(format!("{}_CACHE", BINARY_NAME.clone()))
        .ok()
        .map(PathBuf::from)
});
static DATA_DIR: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    env::var(format!("{}_DATA", BINARY_NAME.clone()))
        .ok()
        .map(PathBuf::from)
});

fn project_directory() -> Option<ProjectDirs> {
    ProjectDirs::from("com", &PACKAGE_NAME, &BINARY_NAME)
}

pub fn get_config_dir() -> PathBuf {
    CONFIG_DIR.clone().unwrap_or_else(|| {
        project_directory().map_or_else(
            || PathBuf::from(".").join(".config"),
            |proj_dirs| proj_dirs.config_local_dir().to_path_buf(),
        )
    })
}

pub fn get_cache_dir() -> PathBuf {
    CACHE_DIR.clone().unwrap_or_else(|| {
        project_directory().map_or_else(
            || PathBuf::from(".").join(".cache"),
            |proj_dirs| proj_dirs.cache_dir().to_path_buf(),
        )
    })
}

pub fn get_data_dir() -> PathBuf {
    DATA_DIR.clone().unwrap_or_else(|| {
        project_directory().map_or_else(
            || PathBuf::from(".").join(".data"),
            |proj_dirs| proj_dirs.data_local_dir().to_path_buf(),
        )
    })
}

pub fn persist_config(path: &Path, config: &CoreConfig) -> Result<()> {
    let toml_string = toml::to_string(&config)?;

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);

    // The config holds the OAuth token, other users on the machine must not read it.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);

    let mut file = options.open(path)?;

    file.write_all(toml_string.as_bytes())?;
    drop(file);

    Ok(())
}

const RAW_DEFAULT_CONFIG_URL: &str =
    "https://raw.githubusercontent.com/Xithrius/twitch-tui/main/default-config.toml";

pub fn persist_default_config(path: &Path) -> Result<()> {
    let default_config = task::block_in_place(move || {
        Handle::current()
            .block_on(async move { reqwest::get(RAW_DEFAULT_CONFIG_URL).await?.text().await })
    })?;

    let mut file = File::create(path)?;

    file.write_all(default_config.as_bytes())?;
    drop(file);

    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn config_with_token_is_private_to_the_user() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        persist_config(&path, &CoreConfig::default()).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
