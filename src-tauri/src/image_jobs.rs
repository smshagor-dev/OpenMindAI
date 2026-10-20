//! Track image generation jobs separately from chat completions.
//! An image card can stop its own renderer without terminating a different
//! conversation's generation or the shared llama.cpp chat server.

use std::{collections::HashMap, sync::Mutex};

use tokio::sync::{Semaphore, SemaphorePermit};
use tokio_util::sync::CancellationToken;

use crate::app_error::AppError;

pub struct ActiveImageJobs {
    jobs: Mutex<HashMap<String, CancellationToken>>,
    render_slot: Semaphore,
}

impl Default for ActiveImageJobs {
    fn default() -> Self {
        Self {
            jobs: Mutex::new(HashMap::new()),
            // Running several SDXL children in parallel can exhaust GPU and
            // system memory. All image jobs share one local renderer slot.
            render_slot: Semaphore::new(1),
        }
    }
}

pub struct ImageJobGuard<'a> {
    registry: &'a ActiveImageJobs,
    artifact_id: String,
    cancellation: CancellationToken,
}

impl ActiveImageJobs {
    pub fn try_reserve_renderer(&self) -> Option<SemaphorePermit<'_>> {
        self.render_slot.try_acquire().ok()
    }

    pub async fn reserve_renderer(&self) -> Result<SemaphorePermit<'_>, AppError> {
        self.render_slot
            .acquire()
            .await
            .map_err(|_| AppError::internal("image render queue is closed"))
    }

    pub fn start(&self, artifact_id: &str) -> Result<ImageJobGuard<'_>, AppError> {
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| AppError::internal("image job registry lock poisoned"))?;
        if jobs.contains_key(artifact_id) {
            return Err(AppError::internal("image generation is already running"));
        }
        let cancellation = CancellationToken::new();
        jobs.insert(artifact_id.to_string(), cancellation.clone());
        Ok(ImageJobGuard {
            registry: self,
            artifact_id: artifact_id.to_string(),
            cancellation,
        })
    }

    /// Return false if the requested artifact is not currently generating.
    pub fn cancel(&self, artifact_id: &str) -> Result<bool, AppError> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| AppError::internal("image job registry lock poisoned"))?;
        let Some(token) = jobs.get(artifact_id) else {
            return Ok(false);
        };
        token.cancel();
        Ok(true)
    }

    fn finish(&self, artifact_id: &str) {
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.remove(artifact_id);
        }
    }
}

impl ImageJobGuard<'_> {
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
}

impl Drop for ImageJobGuard<'_> {
    fn drop(&mut self) {
        self.registry.finish(&self.artifact_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelling_one_image_does_not_cancel_another() {
        let jobs = ActiveImageJobs::default();
        let first = jobs.start("image-a").unwrap();
        let second = jobs.start("image-b").unwrap();

        assert!(jobs.cancel("image-a").unwrap());
        assert!(first.cancellation().is_cancelled());
        assert!(!second.cancellation().is_cancelled());
        assert!(!jobs.cancel("unknown").unwrap());
    }

    #[tokio::test]
    async fn image_renderer_runs_one_job_at_a_time() {
        let jobs = std::sync::Arc::new(ActiveImageJobs::default());
        let first = jobs.try_reserve_renderer().expect("first image slot");
        assert!(jobs.try_reserve_renderer().is_none());

        let next_jobs = jobs.clone();
        let (started, next_started) = tokio::sync::oneshot::channel();
        let second = tokio::spawn(async move {
            let _permit = next_jobs.reserve_renderer().await.unwrap();
            let _ = started.send(());
        });
        tokio::task::yield_now().await;
        assert!(!second.is_finished(), "queued job started while a render was running");

        drop(first);
        next_started.await.expect("second image runs after first finishes");
        second.await.unwrap();
        assert!(jobs.try_reserve_renderer().is_some());
    }

    #[tokio::test]
    async fn cancelling_a_queued_image_does_not_consume_the_next_slot() {
        let jobs = std::sync::Arc::new(ActiveImageJobs::default());
        let running = jobs.try_reserve_renderer().expect("running image slot");
        let queued = jobs.start("waiting-image").unwrap();
        let cancellation = queued.cancellation().clone();
        let next_jobs = jobs.clone();
        let waiting = tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => false,
                result = next_jobs.reserve_renderer() => {
                    let _permit = result.unwrap();
                    true
                }
            }
        });
        assert!(jobs.cancel("waiting-image").unwrap());
        assert!(!waiting.await.unwrap());
        drop(running);
        assert!(jobs.try_reserve_renderer().is_some());
    }

    #[test]
    fn finishing_an_image_releases_its_registration() {
        let jobs = ActiveImageJobs::default();
        {
            let _running = jobs.start("image-a").unwrap();
            assert!(jobs.start("image-a").is_err());
        }
        assert!(!jobs.cancel("image-a").unwrap());
        let restarted = jobs.start("image-a").unwrap();
        assert!(!restarted.cancellation().is_cancelled());
    }
}
