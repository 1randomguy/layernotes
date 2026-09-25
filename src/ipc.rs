//! IPC via Unix domain socket.
//!
//! The daemon listens on `$XDG_RUNTIME_DIR/layernotes.sock`.
//! The same binary acts as a client via `layernotes msg <command>`.

use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow};
use clap::Subcommand;
use iced::Subscription;

use crate::xdg;

/// Maximum bytes to read from a client connection.
const MAX_REQUEST_LEN: u64 = 4096;

/// IPC command that can be sent to the daemon.
#[derive(Subcommand, Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcCommand {
    /// Toggle the notes between the Bottom and Top layers
    ToggleLayer,
    /// Create a new note on the primary output and start editing it
    AddNote,
}

impl fmt::Display for IpcCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IpcCommand::ToggleLayer => write!(f, "toggle-layer"),
            IpcCommand::AddNote => write!(f, "add-note"),
        }
    }
}

impl FromStr for IpcCommand {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "toggle-layer" => Ok(IpcCommand::ToggleLayer),
            "add-note" => Ok(IpcCommand::AddNote),
            _ => Err(anyhow!("unknown IPC command: {s:?}")),
        }
    }
}

pub fn socket_path() -> PathBuf {
    match xdg::runtime_dir() {
        Some(dir) => dir.join("layernotes.sock"),
        None => std::env::temp_dir().join("layernotes.sock"),
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Run the IPC client: connect to the daemon, send a command, print the response.
pub fn run_client(cmd: &IpcCommand) -> Result<()> {
    let path = socket_path();
    let mut stream = UnixStream::connect(&path)
        .with_context(|| format!("connect to {} — is layernotes running?", path.display()))?;

    let line = format!("{cmd}\n");
    stream.write_all(line.as_bytes()).context("send command")?;
    stream.flush()?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let mut response = String::new();
    BufReader::new((&stream).take(MAX_REQUEST_LEN))
        .read_line(&mut response)
        .context("read response")?;
    let response = response.trim_end();

    if let Some(err) = response.strip_prefix("error ") {
        return Err(anyhow!("{err}"));
    }

    if !response.is_empty() {
        println!("{response}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

enum ListenerError {
    /// Another layernotes instance is already listening on the socket.
    AlreadyRunning,
    Other(anyhow::Error),
}

/// Create the Unix listener, taking care not to steal a live server's socket.
///
/// The socket path is shared across instances, so we probe it first: a
/// successful connect means a primary is already serving and we must not
/// remove the file or bind a new listener — otherwise we'd orphan the
/// primary's fd and break `layernotes msg` until it's restarted.
fn create_listener() -> std::result::Result<UnixListener, ListenerError> {
    let path = socket_path();

    match UnixStream::connect(&path) {
        Ok(_) => return Err(ListenerError::AlreadyRunning),
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            if let Err(e) = std::fs::remove_file(&path)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                return Err(ListenerError::Other(
                    anyhow::Error::new(e)
                        .context(format!("remove stale socket {}", path.display())),
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(ListenerError::Other(
                anyhow::Error::new(e).context(format!("probe socket {}", path.display())),
            ));
        }
    }

    let listener = UnixListener::bind(&path)
        .with_context(|| format!("bind {}", path.display()))
        .map_err(ListenerError::Other)?;
    listener
        .set_nonblocking(true)
        .context("set_nonblocking")
        .map_err(ListenerError::Other)?;
    log::info!("IPC listening on {}", path.display());
    Ok(listener)
}

/// Read a single command from an accepted client connection.
fn read_request(stream: &UnixStream) -> Result<IpcCommand> {
    let mut line = String::new();
    BufReader::new(stream.take(MAX_REQUEST_LEN))
        .read_line(&mut line)
        .context("read IPC command")?;
    line.trim().parse()
}

/// Write a response line to the client.
fn write_response(stream: &mut UnixStream, response: &str) {
    let msg = format!("{response}\n");
    if let Err(e) = stream.write_all(msg.as_bytes()) {
        log::debug!("IPC write response failed: {e}");
    }
}

/// Handle a single accepted client connection.
fn handle_connection(mut stream: UnixStream) -> Option<IpcCommand> {
    match read_request(&stream) {
        Ok(cmd) => {
            write_response(&mut stream, "ok");
            Some(cmd)
        }
        Err(e) => {
            write_response(&mut stream, &format!("error {e:#}"));
            None
        }
    }
}

fn init_listener() -> Option<tokio::net::UnixListener> {
    let std_listener = match create_listener() {
        Ok(l) => l,
        Err(ListenerError::AlreadyRunning) => {
            log::warn!(
                "another layernotes instance owns the IPC socket; this instance will run without IPC"
            );
            return None;
        }
        Err(ListenerError::Other(e)) => {
            log::error!("Failed to create IPC listener: {e:#}");
            return None;
        }
    };
    match tokio::net::UnixListener::from_std(std_listener) {
        Ok(l) => Some(l),
        Err(e) => {
            log::error!("Failed to convert IPC listener to tokio: {e}");
            None
        }
    }
}

/// Subscription that listens for IPC commands on the Unix socket.
pub fn subscription() -> Subscription<IpcCommand> {
    use iced::futures::StreamExt;

    Subscription::run(|| {
        iced::futures::stream::unfold(None::<tokio::net::UnixListener>, |listener| async {
            let listener = match listener {
                Some(l) => l,
                None => init_listener()?,
            };
            let (request, listener) = match listener.accept().await {
                Ok((stream, _)) => {
                    let request = match stream.into_std() {
                        Ok(std_stream) => handle_connection(std_stream),
                        Err(e) => {
                            log::error!("IPC stream conversion error: {e}");
                            None
                        }
                    };
                    (request, listener)
                }
                Err(e) => {
                    log::error!("IPC accept error: {e}");
                    (None, listener)
                }
            };
            Some((request, Some(listener)))
        })
        .filter_map(iced::futures::future::ready)
    })
}
