//! Track image generation jobs separately from chat completions.
//! An image card can stop its own renderer without terminating a different
//! conversation's generation or the shared llama.cpp chat server.

use std::{collections::HashMap, sync::Mutex};

use tokio::sync::{Semaphore, SemaphorePermit};
use tokio_util::sync::CancellationToken;

use crate::app_error::AppError;

struct MediaJob {
    token: CancellationToken,
    conversation_id: Option<String>,
}

pub struct ActiveImageJobs {
    jobs: Mutex<HashMap<String, MediaJob>>,
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
        self.start_scoped(artifact_id, None)
    }

    /// Register media rendering against a conversation while retaining the
    /// per-artifact Stop action. An unrelated conversation cannot cancel it.
    pub fn start_scoped(
        &self,
        artifact_id: &str,
        conversation_id: Option<&str>,
    ) -> Result<ImageJobGuard<'_>, AppError> {
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| AppError::internal("image job registry lock poisoned"))?;
        if jobs.contains_key(artifact_id) {
            return Err(AppError::internal("image generation is already running"));
        }
        let cancellation = CancellationToken::new();
        jobs.insert(
            artifact_id.to_string(),
            MediaJob {
                token: cancellation.clone(),
                conversation_id: conversation_id.map(ToString::to_string),
            },
        );
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
        token.token.cancel();
        Ok(true)
    }

    /// Stop only jobs actually registered to this conversation. No global
    /// token is shared between independent chat/video tasks.
    pub fn cancel_conversation(&self, conversation_id: &str) -> Result<usize, AppError> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| AppError::internal("media job registry lock poisoned"))?;
        let mut cancelled = 0;
        for job in jobs.values() {
            if job.conversation_id.as_deref() == Some(conversation_id) {
                job.token.cancel();
                cancelled += 1;
            }
        }
        Ok(cancelled)
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
    fn stop_conversation_cancels_its_video_but_not_other_conversations() {
        let jobs = ActiveImageJobs::default();
        let image_a = jobs.start_scoped("image-a", Some("chat-a")).unwrap();
        let video_a = jobs.start_scoped("video-a", Some("chat-a")).unwrap();
        let video_b = jobs.start_scoped("video-b", Some("chat-b")).unwrap();
        let unscoped = jobs.start("legacy-image").unwrap();
        assert_eq!(jobs.cancel_conversation("chat-a").unwrap(), 2);
        assert!(image_a.cancellation().is_cancelled());
        assert!(video_a.cancellation().is_cancelled());
        assert!(!video_b.cancellation().is_cancelled());
        assert!(!unscoped.cancellation().is_cancelled());
        assert_eq!(jobs.cancel_conversation("missing").unwrap(), 0);
    }

    #[test]
    fn cancelled_video_registration_cannot_cancel_later_jobs_after_drop() {
        let jobs = ActiveImageJobs::default();
        let first = jobs.start_scoped("video-1", Some("chat-a")).unwrap();
        assert_eq!(jobs.cancel_conversation("chat-a").unwrap(), 1);
        assert!(first.cancellation().is_cancelled());
        drop(first);
        let second = jobs.start_scoped("video-1", Some("chat-b")).unwrap();
        assert!(!second.cancellation().is_cancelled());
        assert_eq!(jobs.cancel_conversation("chat-a").unwrap(), 0);
        assert_eq!(jobs.cancel_conversation("chat-b").unwrap(), 1);
        assert!(second.cancellation().is_cancelled());
    }

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
        assert!(
            !second.is_finished(),
            "queued job started while a render was running"
        );

        drop(first);
        next_started
            .await
            .expect("second image runs after first finishes");
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
