use std::env;
use std::os::linux::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// `$XDG_RUNTIME_DIR`, validated to be an absolute directory owned by the user
/// with mode 0700 (as required by the XDG base directory specification).
pub fn get_runtime_dir() -> Option<PathBuf> {
    let runtime_dir = PathBuf::from(env::var_os("XDG_RUNTIME_DIR")?);
    let metadata = runtime_dir.metadata().ok()?;
    let uid = unsafe { libc::geteuid() };
    (runtime_dir.is_absolute()
        && metadata.is_dir()
        && metadata.st_uid() == uid
        && metadata.permissions().mode() & 0o777 == 0o700)
        .then_some(runtime_dir)
}

pub fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

pub fn config_dir() -> PathBuf {
    if let Some(dir) = env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(dir)
    } else if let Some(home) = home_dir() {
        home.join(".config")
    } else {
        PathBuf::from(".config")
    }
}

pub fn data_dir() -> PathBuf {
    if let Some(dir) = env::var_os("XDG_DATA_HOME") {
        PathBuf::from(dir)
    } else if let Some(home) = home_dir() {
        home.join(".local/share")
    } else {
        PathBuf::from(".local/share")
    }
}

/// Expand a leading `~` using `$HOME`.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = home_dir()
    {
        return home.join(rest);
    }
    if path == "~"
        && let Some(home) = home_dir()
    {
        return home;
    }
    PathBuf::from(path)
}
