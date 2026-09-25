use std::env;
use std::path::PathBuf;

pub fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

/// `$XDG_RUNTIME_DIR`, if it is set to an absolute directory. Used for the IPC
/// socket.
pub fn runtime_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env::var_os("XDG_RUNTIME_DIR")?);
    (dir.is_absolute() && dir.is_dir()).then_some(dir)
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
