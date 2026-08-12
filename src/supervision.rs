use std::future::Future;
use std::time::Duration;

use anyhow::anyhow;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::health::HealthState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CriticalTask {
    Http,
    TelegramBot,
    NotificationWorker,
}

pub struct Supervisor {
    tasks: JoinSet<(CriticalTask, anyhow::Result<()>)>,
    cancellation: CancellationToken,
    health: HealthState,
    shutdown_grace: Duration,
}

impl Supervisor {
    pub fn new(
        cancellation: CancellationToken,
        health: HealthState,
        shutdown_grace: Duration,
    ) -> Self {
        Self {
            tasks: JoinSet::new(),
            cancellation,
            health,
            shutdown_grace,
        }
    }

    pub fn spawn<F>(&mut self, task: CriticalTask, future: F)
    where
        F: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        self.tasks.spawn(async move { (task, future.await) });
    }

    pub async fn run<S>(mut self, shutdown_signal: S) -> anyhow::Result<()>
    where
        S: Future<Output = anyhow::Result<()>> + Send,
    {
        let failure = tokio::select! {
            biased;
            signal = shutdown_signal => {
                signal?;
                None
            }
            result = self.tasks.join_next() => Some(unexpected_exit(result)),
        };

        self.health.begin_shutdown();
        self.cancellation.cancel();

        if tokio::time::timeout(self.shutdown_grace, async {
            while self.tasks.join_next().await.is_some() {}
        })
        .await
        .is_err()
        {
            tracing::warn!("critical tasks did not stop within shutdown grace");
            self.tasks.abort_all();
            while self.tasks.join_next().await.is_some() {}
        }

        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

fn unexpected_exit(
    result: Option<Result<(CriticalTask, anyhow::Result<()>), tokio::task::JoinError>>,
) -> anyhow::Error {
    match result {
        Some(Ok((task, Ok(())))) => anyhow!("critical task {task:?} stopped unexpectedly"),
        Some(Ok((task, Err(error)))) => error.context(format!("critical task {task:?} failed")),
        Some(Err(error)) => anyhow::Error::new(error).context("critical task panicked"),
        None => anyhow!("all critical tasks stopped unexpectedly"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn clean_signal_cancels_tasks() {
        let cancellation = CancellationToken::new();
        let mut supervisor = Supervisor::new(
            cancellation.clone(),
            HealthState::new(false, false),
            Duration::from_secs(1),
        );
        let task_cancellation = cancellation.clone();
        supervisor.spawn(CriticalTask::Http, async move {
            task_cancellation.cancelled().await;
            Ok(())
        });

        supervisor.run(async { Ok(()) }).await.unwrap();
        assert!(cancellation.is_cancelled());
    }

    #[tokio::test]
    async fn unexpected_task_completion_is_fatal() {
        let cancellation = CancellationToken::new();
        let mut supervisor = Supervisor::new(
            cancellation,
            HealthState::new(false, false),
            Duration::from_secs(1),
        );
        supervisor.spawn(CriticalTask::TelegramBot, async { Ok(()) });

        let error = supervisor.run(std::future::pending()).await.unwrap_err();
        assert!(error.to_string().contains("stopped unexpectedly"));
    }
}
