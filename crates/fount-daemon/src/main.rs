use fount_daemon::{EngineHandle, default_socket_path, run_unix_socket_server};
use tracing::{error, info};

#[tokio::main(worker_threads = 2)]
async fn main() {
    tracing_subscriber::fmt::init();

    info!("Starting Fount search daemon...");
    let engine = EngineHandle::spawn();
    let socket_path = default_socket_path();

    let server_engine = engine.clone();
    let server_socket_path = socket_path.clone();

    tokio::select! {
        res = run_unix_socket_server(server_socket_path, server_engine) => {
            if let Err(e) = res {
                error!(error = %e, "Unix domain socket server terminated with error");
            }
        }
        sig = tokio::signal::ctrl_c() => {
            if let Err(e) = sig {
                error!(error = %e, "Failed to listen for Ctrl+C signal");
            } else {
                info!("Received Ctrl+C shutdown signal");
            }
        }
    }

    let _ = std::fs::remove_file(&socket_path);
    let _ = engine.shutdown().await;
    info!("Fount daemon stopped cleanly.");
}
