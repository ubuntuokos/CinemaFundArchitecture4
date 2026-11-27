//! Wake-ups for the scheduler: a connection of its own that `LISTEN`s on
//! [`crate::tasks::CHANNEL`]. Every submit and every state change sends a notification in the same
//! transaction, so it arrives only after the commit. A scheduler waits on [`Listener::wait`] with
//! its own timeout: the timeout is still needed for leases, timed starts and resources that free up
//! outside CFA4.

use std::time::Duration;

use tokio::sync::mpsc;
use tokio_postgres::{AsyncMessage, NoTls};

use crate::StoreError;

#[cfg(test)]
mod tests;

/// Notifications kept until the scheduler reads them (more are folded into one wake-up).
const BUFFER: usize = 1024;

/// Why [`Listener::wait`] returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Wake {
    /// These tasks were added or changed (ids, oldest first, at most 1024).
    Changed(Vec<String>),
    /// Nothing happened within the timeout.
    Timeout,
    /// The connection is gone: connect a new listener (and run a step, something may have been
    /// missed meanwhile).
    Lost,
}

/// A listening connection; dropping it closes the connection.
pub struct Listener {
    rx: mpsc::Receiver<String>,
    _client: tokio_postgres::Client,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Listener {
    pub(crate) async fn connect(config: &tokio_postgres::Config) -> Result<Listener, StoreError> {
        let (client, mut connection) = config.connect(NoTls).await.map_err(StoreError::unavailable)?;
        let (tx, rx) = mpsc::channel(BUFFER);
        // the connection only delivers notifications while it is polled
        let task = tokio::spawn(async move {
            loop {
                match std::future::poll_fn(|cx| connection.poll_message(cx)).await {
                    Some(Ok(AsyncMessage::Notification(n))) => {
                        // a full buffer already guarantees a wake-up; dropping more is harmless
                        let _ = tx.try_send(n.payload().to_owned());
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        log::warn!("cfa4-store listener: {e}");
                        break;
                    }
                    None => break,
                }
            }
        });
        let listener = Listener { rx, _client: client, task };
        listener._client.batch_execute(&format!("LISTEN {}", crate::tasks::CHANNEL)).await.map_err(StoreError::postgres)?;
        Ok(listener)
    }

    /// Wait up to `timeout` for a change; everything that arrived meanwhile comes at once.
    pub async fn wait(&mut self, timeout: Duration) -> Wake {
        let first = match tokio::time::timeout(timeout, self.rx.recv()).await {
            Err(_) => return Wake::Timeout,
            Ok(None) => return Wake::Lost,
            Ok(Some(id)) => id,
        };
        let mut ids = vec![first];
        while let Ok(id) = self.rx.try_recv() {
            if ids.len() >= BUFFER {
                break;
            }
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Wake::Changed(ids)
    }
}
