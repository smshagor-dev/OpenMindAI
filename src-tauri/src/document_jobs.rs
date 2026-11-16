//! Bound CPU-heavy document jobs without occupying async executor threads.
use crate::app_error::AppError;
use std::sync::{Arc, OnceLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();

pub(crate) async fn reserve() -> Result<OwnedSemaphorePermit, AppError> {
    reserve_from(SLOTS.get_or_init(|| Arc::new(Semaphore::new(1))).clone()).await
}
async fn reserve_from(slots: Arc<Semaphore>) -> Result<OwnedSemaphorePermit, AppError> {
    slots.acquire_owned().await.map_err(|_| {
        AppError::ArtifactGenerationFailed("Document export queue is unavailable".into())
    })
}

/// Keep renderer panics inside the worker so completion logic still runs when
/// an IPC caller has disappeared. Panic details are not returned to the UI.
pub(crate) fn render<T>(job: impl FnOnce() -> Result<T, AppError>) -> Result<T, AppError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).unwrap_or_else(|_| {
        Err(AppError::ArtifactGenerationFailed(
            "Document renderer stopped unexpectedly".into(),
        ))
    })
}

/// The worker owns its slot and completion work even if the caller stops waiting.
/// Cancellation of a queued reservation, in contrast, never starts a worker.
pub(crate) async fn run<T: Send + 'static>(
    permit: OwnedSemaphorePermit,
    job: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        job()
    })
    .await
    .map_err(|_| {
        AppError::ArtifactGenerationFailed("Document export worker stopped unexpectedly".into())
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::Duration,
    };
    fn slots() -> Arc<Semaphore> {
        Arc::new(Semaphore::new(1))
    }
    #[tokio::test(flavor = "current_thread")]
    async fn blocked_worker_does_not_block_runtime_progress() {
        let queue = slots();
        let permit = reserve_from(queue).await.unwrap();
        let (started, started_rx) = tokio::sync::oneshot::channel();
        let (release, release_rx) = mpsc::channel();
        let finished = Arc::new(AtomicBool::new(false));
        let flag = finished.clone();
        let task = tokio::spawn(run(permit, move || {
            let _ = started.send(());
            let result = release_rx.recv_timeout(Duration::from_secs(5));
            flag.store(true, Ordering::SeqCst);
            result.map_err(|_| AppError::internal("release timeout"))?;
            Ok(7)
        }));
        started_rx.await.unwrap();
        tokio::task::yield_now().await;
        let progressed_before_finish = !finished.load(Ordering::SeqCst);
        let _ = release.send(());
        let result = task.await.unwrap();
        assert!(
            progressed_before_finish,
            "async task did not progress until blocking job finished"
        );
        assert_eq!(result.unwrap(), 7);
    }
    #[tokio::test]
    async fn only_one_worker_runs_and_queued_reservation_is_async() {
        let queue = slots();
        let permit = reserve_from(queue.clone()).await.unwrap();
        let (started, rx) = tokio::sync::oneshot::channel();
        let (release, blocked) = mpsc::channel();
        let first = tokio::spawn(run(permit, move || {
            let _ = started.send(());
            blocked
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| AppError::internal("release timeout"))?;
            Ok(())
        }));
        rx.await.unwrap();
        let second_queue = queue.clone();
        let entered = Arc::new(AtomicBool::new(false));
        let flag = entered.clone();
        let (queued, queued_rx) = tokio::sync::oneshot::channel();
        let second = tokio::spawn(async move {
            let _ = queued.send(());
            let permit = reserve_from(second_queue).await?;
            run(permit, move || {
                flag.store(true, Ordering::SeqCst);
                Ok(())
            })
            .await
        });
        queued_rx.await.unwrap();
        tokio::task::yield_now().await;
        assert!(!entered.load(Ordering::SeqCst));
        assert!(queue.clone().try_acquire_owned().is_err());
        release.send(()).unwrap();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert!(entered.load(Ordering::SeqCst));
        assert_eq!(queue.available_permits(), 1);
    }
    #[tokio::test]
    async fn cancelling_a_waiting_reservation_leaks_no_slot_or_work() {
        let queue = slots();
        let held = reserve_from(queue.clone()).await.unwrap();
        let other = queue.clone();
        let task = tokio::spawn(async move {
            let permit = reserve_from(other).await?;
            run(permit, || Ok(())).await
        });
        tokio::task::yield_now().await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        drop(held);
        let permit = tokio::time::timeout(Duration::from_secs(2), reserve_from(queue))
            .await
            .unwrap()
            .unwrap();
        drop(permit);
    }
    #[tokio::test]
    async fn detached_running_worker_retains_slot_until_completion() {
        let queue = slots();
        let permit = reserve_from(queue.clone()).await.unwrap();
        let (started, rx) = tokio::sync::oneshot::channel();
        let (release, blocked) = mpsc::channel();
        let (done, done_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(run(permit, move || {
            let _ = started.send(());
            blocked
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| AppError::internal("release timeout"))?;
            let _ = done.send(());
            Ok(())
        }));
        rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(queue.clone().try_acquire_owned().is_err());
        release.send(()).unwrap();
        done_rx.await.unwrap();
        let next = tokio::time::timeout(Duration::from_secs(2), reserve_from(queue))
            .await
            .unwrap()
            .unwrap();
        drop(next);
    }
    #[tokio::test]
    async fn panic_is_redacted_and_releases_slot() {
        let queue = slots();
        let permit = reserve_from(queue.clone()).await.unwrap();
        let error = run::<()>(permit, || panic!("synthetic-private-content"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("stopped unexpectedly"));
        assert!(!error.to_string().contains("synthetic-private-content"));
        assert_eq!(queue.available_permits(), 1);
    }
    #[tokio::test]
    async fn ordinary_generation_error_is_preserved_and_releases_slot() {
        let queue = slots();
        let permit = reserve_from(queue.clone()).await.unwrap();
        let error = run::<()>(permit, || {
            Err(AppError::ArtifactGenerationFailed("disk failure".into()))
        })
        .await
        .unwrap_err();
        assert!(
            matches!(error,AppError::ArtifactGenerationFailed(message) if message=="disk failure")
        );
        assert_eq!(queue.available_permits(), 1);
    }
    #[tokio::test]
    async fn renderer_panic_is_a_result_and_completion_still_runs() {
        let queue = slots();
        let permit = reserve_from(queue.clone()).await.unwrap();
        let finalized = Arc::new(AtomicBool::new(false));
        let flag = finalized.clone();
        let result = run(permit, move || {
            let outcome = render::<()>(|| panic!("synthetic-renderer-private-text"));
            flag.store(true, Ordering::SeqCst);
            outcome
        })
        .await;
        assert!(finalized.load(Ordering::SeqCst));
        let error = result.unwrap_err().to_string();
        assert!(error.contains("renderer stopped"));
        assert!(!error.contains("synthetic-renderer-private-text"));
        assert_eq!(queue.available_permits(), 1);
    }

    #[tokio::test]
    async fn global_reservation_accepts_a_completed_job() {
        let permit = reserve().await.unwrap();
        assert_eq!(run(permit, || Ok(42)).await.unwrap(), 42);
    }
}
