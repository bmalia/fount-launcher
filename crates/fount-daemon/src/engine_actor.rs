use crate::protocol::{ClientMessage, DaemonMessage, SearchResultWire};
use fount_core::{IndexCatalog, LauncherConfig, SearchResult};
use std::thread;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn};

/// Errors returned when communicating with the background search engine actor.
#[derive(Debug, Error)]
pub enum EngineError {
    #[error("Search engine actor channel closed")]
    ActorStopped,
    #[error("Failed to launch entry: {0}")]
    LaunchFailed(String),
}

/// Commands sent from async Tokio tasks across the thread boundary to the synchronous actor thread.
pub enum EngineCommand {
    /// Update query, tick Nucleo matcher, and return up to `limit` ranked results.
    Search {
        query: String,
        limit: usize,
        reply: oneshot::Sender<Vec<SearchResult>>,
    },
    /// Record frecency usage for `id` and optionally spawn its underlying action.
    Exec {
        id: String,
        execute_process: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Rescan desktop directories and reload entries.
    Reload { reply: oneshot::Sender<usize> },
    /// Persist frecency state and terminate the actor loop cleanly.
    Shutdown { reply: oneshot::Sender<()> },
}

/// Cloneable async handle used by Tokio IPC servers (Unix socket, D-Bus) to query the actor thread.
#[derive(Clone, Debug)]
pub struct EngineHandle {
    tx: mpsc::Sender<EngineCommand>,
}

impl EngineHandle {
    /// Spawns the dedicated OS actor thread initialized with the default `LauncherConfig`.
    pub fn spawn() -> Self {
        let catalog = IndexCatalog::with_config(LauncherConfig::default());
        Self::spawn_with_catalog(catalog)
    }

    /// Spawns the dedicated OS actor thread taking ownership of the provided `IndexCatalog`.
    ///
    /// `Nucleo` runs its own internal Rayon thread pool; keeping `IndexCatalog` and `nucleo.tick()`
    /// isolated on this dedicated OS thread guarantees Tokio worker threads are never blocked.
    pub fn spawn_with_catalog(mut catalog: IndexCatalog) -> Self {
        let (tx, mut rx) = mpsc::channel::<EngineCommand>(256);

        // Prime initial snapshot
        catalog.tick(10);

        thread::Builder::new()
            .name("fount-engine-actor".into())
            .spawn(move || {
                info!(
                    entries = catalog.len(),
                    "Search engine actor started on dedicated OS thread"
                );

                while let Some(cmd) = rx.blocking_recv() {
                    match cmd {
                        EngineCommand::Search {
                            query,
                            limit,
                            reply,
                        } => {
                            catalog.search(&query);
                            catalog.tick(10);
                            let results = catalog.results(limit);
                            let _ = reply.send(results);
                        }
                        EngineCommand::Exec {
                            id,
                            execute_process,
                            reply,
                        } => {
                            let outcome = match catalog.get_by_id(&id).cloned() {
                                Some(entry) => {
                                    let exec_res = if execute_process {
                                        entry
                                            .action
                                            .execute()
                                            .map(|_| ())
                                            .map_err(|e| e.to_string())
                                    } else {
                                        Ok(())
                                    };

                                    if exec_res.is_ok() {
                                        let new_score = catalog.record_launch(&id);
                                        debug!(id = %id, new_score, "Recorded launch in frecency store");
                                        if let Err(e) = catalog.save_frecency() {
                                            warn!(error = %e, "Failed to save frecency store to disk");
                                        }
                                    }
                                    exec_res
                                }
                                None => Err(format!("Entry not found: '{id}'")),
                            };
                            let _ = reply.send(outcome);
                        }
                        EngineCommand::Reload { reply } => {
                            let count = catalog.reload();
                            catalog.tick(10);
                            info!(entries = count, "Reloaded search catalog");
                            let _ = reply.send(count);
                        }
                        EngineCommand::Shutdown { reply } => {
                            info!("Shutting down search engine actor");
                            if let Err(e) = catalog.save_frecency() {
                                error!(error = %e, "Failed to save frecency state during shutdown");
                            }
                            let _ = reply.send(());
                            break;
                        }
                    }
                }
            })
            .expect("Failed to spawn fount-engine-actor thread");

        Self { tx }
    }

    /// Submits a fuzzy search query to the engine actor and awaits the ranked results.
    pub async fn search(
        &self,
        query: impl Into<String>,
        limit: usize,
    ) -> Result<Vec<SearchResult>, EngineError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(EngineCommand::Search {
                query: query.into(),
                limit,
                reply: reply_tx,
            })
            .await
            .map_err(|_| EngineError::ActorStopped)?;

        reply_rx.await.map_err(|_| EngineError::ActorStopped)
    }

    /// Records a launch for `id` (updating frecency) and optionally spawns the process.
    pub async fn launch(
        &self,
        id: impl Into<String>,
        execute_process: bool,
    ) -> Result<(), EngineError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(EngineCommand::Exec {
                id: id.into(),
                execute_process,
                reply: reply_tx,
            })
            .await
            .map_err(|_| EngineError::ActorStopped)?;

        match reply_rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(msg)) => Err(EngineError::LaunchFailed(msg)),
            Err(_) => Err(EngineError::ActorStopped),
        }
    }

    /// Triggers an index reload and returns the updated entry count.
    pub async fn reload(&self) -> Result<usize, EngineError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(EngineCommand::Reload { reply: reply_tx })
            .await
            .map_err(|_| EngineError::ActorStopped)?;

        reply_rx.await.map_err(|_| EngineError::ActorStopped)
    }

    /// Flushes state to disk and stops the actor thread cleanly.
    pub async fn shutdown(&self) -> Result<(), EngineError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(EngineCommand::Shutdown { reply: reply_tx })
            .await
            .map_err(|_| EngineError::ActorStopped)?;

        reply_rx.await.map_err(|_| EngineError::ActorStopped)
    }

    /// Stateless dispatcher for wire protocol messages received from the IPC server.
    pub async fn dispatch(&self, msg: ClientMessage) -> DaemonMessage {
        match msg {
            ClientMessage::Search { query, limit } => {
                match self.search(query.clone(), limit as usize).await {
                    Ok(results) => DaemonMessage::Results {
                        query,
                        items: results.into_iter().map(SearchResultWire::from).collect(),
                    },
                    Err(e) => DaemonMessage::Error {
                        message: e.to_string(),
                    },
                }
            }
            ClientMessage::Exec { id, execute } => match self.launch(id.clone(), execute).await {
                Ok(()) => DaemonMessage::Launched {
                    id,
                    success: true,
                    error: None,
                },
                Err(e) => DaemonMessage::Launched {
                    id,
                    success: false,
                    error: Some(e.to_string()),
                },
            },
            ClientMessage::Reload => match self.reload().await {
                Ok(count) => DaemonMessage::Reloaded {
                    count: count as u32,
                },
                Err(e) => DaemonMessage::Error {
                    message: e.to_string(),
                },
            },
            ClientMessage::Ping => DaemonMessage::Pong,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fount_core::FrecencyStore;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_actor_search_launch_and_frecency() {
        let temp = tempdir().unwrap();
        let apps_dir = temp.path().join("applications");
        let frecency_path = temp.path().join("frecency.bin");
        fs::create_dir_all(&apps_dir).unwrap();

        fs::write(
            apps_dir.join("code.desktop"),
            r#"
[Desktop Entry]
Type=Application
Name=Code Editor
Exec=code
"#,
        )
        .unwrap();

        fs::write(
            apps_dir.join("console.desktop"),
            r#"
[Desktop Entry]
Type=Application
Name=Console Terminal
Exec=kgx
"#,
        )
        .unwrap();

        let mut catalog = IndexCatalog::with_dirs(vec![apps_dir.clone()]);
        catalog.set_frecency_store(FrecencyStore::open_or_create(frecency_path.clone()));
        catalog.set_include_system_actions(true);

        let handle = EngineHandle::spawn_with_catalog(catalog);

        // 1. Search for "code"
        let results = handle.search("code", 5).await.unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].entry.id, "code.desktop");

        // 2. Search for system action by alias "shutdown"
        let sys_results = handle.search("shutdown", 5).await.unwrap();
        assert!(!sys_results.is_empty());
        assert_eq!(sys_results[0].entry.id, "action:poweroff");

        // 3. Record multiple launches for "console.desktop" without spawning external binary
        for _ in 0..5 {
            handle.launch("console.desktop", false).await.unwrap();
        }
        assert!(frecency_path.is_file());

        // 4. Empty query should rank "console.desktop" #1 due to frecency
        let recent = handle.search("", 5).await.unwrap();
        assert_eq!(recent[0].entry.id, "console.desktop");

        // 5. Test wire protocol dispatch
        let resp = handle
            .dispatch(ClientMessage::Search {
                query: "co".into(),
                limit: 5,
            })
            .await;
        match resp {
            DaemonMessage::Results { items, .. } => {
                assert!(!items.is_empty());
                assert_eq!(items[0].id, "console.desktop");
            }
            other => panic!("Unexpected daemon response: {other:?}"),
        }

        // 6. Test reload and clean shutdown
        let count = handle.reload().await.unwrap();
        assert_eq!(count, 7); // 2 desktop apps + 5 system actions

        handle.shutdown().await.unwrap();
    }
}
