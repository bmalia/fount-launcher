use fount_daemon::EngineHandle;
use tracing::info;

#[tokio::main(worker_threads = 2)]
async fn main() {
    tracing_subscriber::fmt::init();

    info!("Starting Fount search daemon...");
    let engine = EngineHandle::spawn();

    info!("Engine actor running. Waiting for shutdown signal (Ctrl+C)...");
    if let Err(e) = tokio::signal::ctrl_c().await {
        tracing::error!(error = %e, "Failed to listen for Ctrl+C signal");
    }

    let _ = engine.shutdown().await;
    info!("Shutdown cleanly. Exiting.");
}
