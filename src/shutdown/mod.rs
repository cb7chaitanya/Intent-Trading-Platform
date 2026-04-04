use std::time::Duration;

use tokio_util::sync::CancellationToken;

const GRACEFUL_TIMEOUT_SECS: u64 = 30;

/// Shared shutdown coordinator.
#[derive(Clone)]
pub struct Shutdown {
    token: CancellationToken,
}

impl Shutdown {
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    /// Returns a child token that tasks can select! on.
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    /// Returns true once shutdown has been triggered.
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Trigger shutdown. All tokens become cancelled.
    pub fn trigger(&self) {
        tracing::info!("Shutdown triggered");
        self.token.cancel();
    }

    /// Wait for a SIGINT or SIGTERM signal, then trigger shutdown.
    pub async fn listen_for_signals(&self) {
        let ctrl_c = async {
            tokio::signal::ctrl_c()
                .await
                .expect("Failed to install CTRL+C handler");
        };

        #[cfg(unix)]
        let terminate = async {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("Failed to install SIGTERM handler")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => tracing::info!("Received SIGINT"),
            _ = terminate => tracing::info!("Received SIGTERM"),
        }

        self.trigger();
    }

    /// Wait for all tasks to finish, with a hard timeout.
    pub async fn wait_for_completion(&self, task_handles: Vec<tokio::task::JoinHandle<()>>) {
        tracing::info!(
            tasks = task_handles.len(),
            timeout_secs = GRACEFUL_TIMEOUT_SECS,
            "Waiting for in-flight tasks to complete"
        );

        let deadline = tokio::time::sleep(Duration::from_secs(GRACEFUL_TIMEOUT_SECS));
        tokio::pin!(deadline);

        let join_all = async {
            for handle in task_handles {
                let _ = handle.await;
            }
        };

        tokio::select! {
            _ = join_all => {
                tracing::info!("All tasks completed gracefully");
            }
            _ = &mut deadline => {
                tracing::warn!("Graceful shutdown timed out, forcing exit");
            }
        }
    }
}
