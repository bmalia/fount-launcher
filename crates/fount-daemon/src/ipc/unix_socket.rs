use crate::engine_actor::EngineHandle;
use crate::protocol::{ClientMessage, DaemonMessage};
use std::env;
use std::io;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, info, warn};

/// Maximum allowed frame size (16 MiB) to protect against malformed length headers.
const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Wire encoding detected for a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WireFormat {
    Bincode,
    Json,
}

/// Resolves the default Unix Domain Socket path:
/// `$XDG_RUNTIME_DIR/fount.sock` (fallback `/tmp/fount.sock`).
pub fn default_socket_path() -> PathBuf {
    if let Ok(runtime_dir) = env::var("XDG_RUNTIME_DIR")
        && !runtime_dir.is_empty()
    {
        return PathBuf::from(runtime_dir).join("fount.sock");
    }
    PathBuf::from("/tmp/fount.sock")
}

/// Reads a single 4-byte big-endian length-delimited frame from an async reader.
/// Returns `Ok(None)` on clean EOF.
pub async fn read_frame<R: AsyncReadExt + Unpin>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }

    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Frame size {len} exceeds maximum allowed ({MAX_FRAME_SIZE} bytes)"),
        ));
    }

    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    Ok(Some(payload))
}

/// Writes a single 4-byte big-endian length-delimited frame to an async writer.
pub async fn write_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    payload: &[u8],
) -> io::Result<()> {
    let len = u32::try_from(payload.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Payload exceeds u32::MAX bytes",
        )
    })?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(payload).await?;
    writer.flush().await
}

/// Decodes a `ClientMessage` from a raw frame, supporting `bincode` (primary) and `serde_json` (fallback).
fn decode_request(bytes: &[u8]) -> Result<(ClientMessage, WireFormat), String> {
    // Check if payload looks like JSON (starts with whitespace, '{', or '"')
    let trimmed = bytes.iter().copied().find(|b| !b.is_ascii_whitespace());
    if matches!(trimmed, Some(b'{') | Some(b'"'))
        && let Ok(msg) = serde_json::from_slice::<ClientMessage>(bytes)
    {
        return Ok((msg, WireFormat::Json));
    }

    // Default: high-speed bincode
    match bincode::deserialize::<ClientMessage>(bytes) {
        Ok(msg) => Ok((msg, WireFormat::Bincode)),
        Err(bin_err) => {
            // Try JSON fallback just in case
            if let Ok(msg) = serde_json::from_slice::<ClientMessage>(bytes) {
                Ok((msg, WireFormat::Json))
            } else {
                Err(format!("Failed to decode ClientMessage: {bin_err}"))
            }
        }
    }
}

/// Encodes a `DaemonMessage` using the requested wire format.
fn encode_response(msg: &DaemonMessage, format: WireFormat) -> io::Result<Vec<u8>> {
    match format {
        WireFormat::Bincode => bincode::serialize(msg).map_err(io::Error::other),
        WireFormat::Json => serde_json::to_vec(msg).map_err(io::Error::other),
    }
}

/// Binds a `UnixListener` to `socket_path` (cleaning up any stale socket file first).
pub fn bind_unix_listener(socket_path: &Path) -> io::Result<UnixListener> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }

    UnixListener::bind(socket_path)
}

/// Runs the Unix Domain Socket IPC server loop using an already-bound `UnixListener`.
pub async fn serve_unix_listener(listener: UnixListener, engine: EngineHandle) -> io::Result<()> {
    loop {
        let (stream, _addr) = listener.accept().await?;
        let engine_clone = engine.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_client_connection(stream, engine_clone).await {
                debug!(error = %e, "Unix socket client connection ended with error");
            }
        });
    }
}

/// Binds and runs the Unix Domain Socket IPC server at `socket_path`.
pub async fn run_unix_socket_server(
    socket_path: PathBuf,
    engine: EngineHandle,
) -> io::Result<()> {
    let listener = bind_unix_listener(&socket_path)?;
    info!(path = %socket_path.display(), "Unix domain socket server listening");
    serve_unix_listener(listener, engine).await
}

/// Handles a single persistent client connection, processing length-delimited request streams.
async fn handle_client_connection(mut stream: UnixStream, engine: EngineHandle) -> io::Result<()> {
    while let Some(frame) = read_frame(&mut stream).await? {
        match decode_request(&frame) {
            Ok((request, format)) => {
                let response = engine.dispatch(request).await;
                let encoded = encode_response(&response, format)?;
                write_frame(&mut stream, &encoded).await?;
            }
            Err(err_msg) => {
                warn!(error = %err_msg, "Malformed request frame on Unix socket");
                let err_resp = DaemonMessage::Error { message: err_msg };
                let encoded = encode_response(&err_resp, WireFormat::Bincode)?;
                write_frame(&mut stream, &encoded).await?;
            }
        }
    }
    Ok(())
}

/// High-speed async client for communicating with the daemon over a Unix Domain Socket.
pub struct UnixSocketClient {
    stream: UnixStream,
}

impl UnixSocketClient {
    /// Connects to the daemon at `socket_path`.
    pub async fn connect<P: AsRef<Path>>(socket_path: P) -> io::Result<Self> {
        let stream = UnixStream::connect(socket_path).await?;
        Ok(Self { stream })
    }

    /// Sends a `ClientMessage` using `bincode` framing and awaits the `DaemonMessage` response.
    pub async fn send(&mut self, msg: &ClientMessage) -> io::Result<DaemonMessage> {
        let payload = bincode::serialize(msg).map_err(io::Error::other)?;
        write_frame(&mut self.stream, &payload).await?;

        let resp_bytes = read_frame(&mut self.stream).await?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Daemon closed connection before sending response",
            )
        })?;

        bincode::deserialize(&resp_bytes).map_err(io::Error::other)
    }

    /// Sends a `ClientMessage` serialized as JSON and awaits the JSON `DaemonMessage` response.
    pub async fn send_json(&mut self, msg: &ClientMessage) -> io::Result<DaemonMessage> {
        let payload = serde_json::to_vec(msg).map_err(io::Error::other)?;
        write_frame(&mut self.stream, &payload).await?;

        let resp_bytes = read_frame(&mut self.stream).await?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Daemon closed connection before sending response",
            )
        })?;

        serde_json::from_slice(&resp_bytes).map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fount_core::{FrecencyStore, IndexCatalog};
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_unix_socket_bincode_and_json_streaming() {
        let temp = tempdir().unwrap();
        let apps_dir = temp.path().join("applications");
        let frecency_path = temp.path().join("frecency.bin");
        let sock_path = temp.path().join("fount-test.sock");
        fs::create_dir_all(&apps_dir).unwrap();

        fs::write(
            apps_dir.join("firefox.desktop"),
            r#"
[Desktop Entry]
Type=Application
Name=Firefox Web Browser
Exec=firefox
Icon=firefox
"#,
        )
        .unwrap();

        fs::write(
            apps_dir.join("files.desktop"),
            r#"
[Desktop Entry]
Type=Application
Name=Files
Exec=nautilus
Icon=system-file-manager
"#,
        )
        .unwrap();

        let mut catalog = IndexCatalog::with_dirs(vec![apps_dir]);
        catalog.set_frecency_store(FrecencyStore::open_or_create(frecency_path));
        catalog.set_include_system_actions(true);

        let engine = EngineHandle::spawn_with_catalog(catalog);
        let listener = bind_unix_listener(&sock_path).unwrap();

        let server_engine = engine.clone();
        let server_task = tokio::spawn(async move {
            let _ = serve_unix_listener(listener, server_engine).await;
        });

        // Connect client
        let mut client = UnixSocketClient::connect(&sock_path).await.unwrap();

        // 1. Ping -> Pong
        let pong = client.send(&ClientMessage::Ping).await.unwrap();
        assert_eq!(pong, DaemonMessage::Pong);

        // 2. Simulate rapid keystroke streaming over a single persistent socket connection
        for keystroke in ["f", "fi", "fir", "fire"] {
            let resp = client
                .send(&ClientMessage::Search {
                    query: keystroke.into(),
                    limit: 5,
                })
                .await
                .unwrap();

            match resp {
                DaemonMessage::Results { query, items } => {
                    assert_eq!(query, keystroke);
                    assert!(!items.is_empty());
                }
                other => panic!("Expected Results, got {other:?}"),
            }
        }

        // 3. Exec "files.desktop" (record frecency without spawning process)
        let launch_resp = client
            .send(&ClientMessage::Exec {
                id: "files.desktop".into(),
                execute: false,
            })
            .await
            .unwrap();
        assert_eq!(
            launch_resp,
            DaemonMessage::Launched {
                id: "files.desktop".into(),
                success: true,
                error: None,
            }
        );

        // 4. Test JSON framing fallback on the same socket
        let json_resp = client
            .send_json(&ClientMessage::Search {
                query: "fi".into(),
                limit: 5,
            })
            .await
            .unwrap();
        match json_resp {
            DaemonMessage::Results { items, .. } => {
                // "files.desktop" was launched, so frecency boosts it to #1 for "fi"!
                assert_eq!(items[0].id, "files.desktop");
            }
            other => panic!("Expected Results from JSON request, got {other:?}"),
        }

        server_task.abort();
        engine.shutdown().await.unwrap();
    }
}
