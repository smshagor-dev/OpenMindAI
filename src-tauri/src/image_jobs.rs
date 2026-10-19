//! Track image generation jobs separately from chat completions.
//! An image card can stop its own renderer without terminating a different
//! conversation's generation or the shared llama.cpp chat server.

use std::{collections::HashMap, sync::Mutex};

use tokio_util::sync::CancellationToken;

use crate::app_error::AppError;

#[derive(Default)]
pub struct ActiveImageJobs {
    jobs: Mutex<HashMap<String, CancellationToken>>,
}

pub struct ImageJobGuard<'a> {
    registry: &'a ActiveImageJobs,
    artifact_id: String,
    cancellation: CancellationToken,
}

impl ActiveImageJobs {
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
