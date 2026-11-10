//! Coordinate refreshes for each global credential store. A waiter observes the
//! leader's result; a later request can retry a failed attempt. Never log tokens.
use crate::app_error::AppError;
use std::{
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
    time::Duration,
};
use tokio::sync::Mutex;

pub(crate) enum Store {
    Microsoft,
    Slack,
    Notion,
    Dropbox,
    GoogleWorkspace,
    GoogleConnected,
}
#[derive(Default)]
struct Flight {
    generation: AtomicU64,
    result: Mutex<Option<Result<String, String>>>,
}
impl Flight {
    async fn run<F, Fut>(&self, operation: F) -> Result<String, AppError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<String, AppError>>,
    {
        let observed = self.generation.load(Ordering::Acquire);
        let mut result = tokio::time::timeout(Duration::from_secs(95), self.result.lock())
            .await
            .map_err(|_| {
                AppError::internal("Timed out waiting for OAuth refresh; try again later")
            })?;
        if self.generation.load(Ordering::Acquire) != observed {
            return result
                .as_ref()
                .expect("completed refresh has a result")
                .clone()
                .map_err(AppError::internal);
        }
        // The caller re-reads expiry and credentials while holding this guard.
        let refreshed = operation().await;
        *result = Some(
            refreshed
                .as_ref()
                .map(Clone::clone)
                .map_err(ToString::to_string),
        );
        self.generation.fetch_add(1, Ordering::Release);
        refreshed
    }
}

pub(crate) async fn coordinated<F, Fut>(store: Store, operation: F) -> Result<String, AppError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<String, AppError>>,
{
    static FLIGHTS: OnceLock<[Flight; 6]> = OnceLock::new();
    let index = match store {
        Store::Microsoft => 0,
        Store::Slack => 1,
        Store::Notion => 2,
        Store::Dropbox => 3,
        Store::GoogleWorkspace => 4,
        Store::GoogleConnected => 5,
    };
    FLIGHTS.get_or_init(|| std::array::from_fn(|_| Flight::default()))[index]
        .run(operation)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicUsize, Arc};
    #[tokio::test]
    async fn concurrent_callers_share_one_rotating_refresh_and_persisted_token() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let rotations = Arc::new(AtomicUsize::new(0));
        let server = {
            let rotations = rotations.clone();
            tokio::spawn(async move {
                loop {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let rotations = rotations.clone();
                    tokio::spawn(async move {
                        let mut request = Vec::new();
                        loop {
                            let mut chunk = [0; 1024];
                            let read = socket.read(&mut chunk).await.unwrap();
                            assert!(read > 0);
                            request.extend_from_slice(&chunk[..read]);
                            if let Some(end) =
                                request.windows(4).position(|part| part == b"\r\n\r\n")
                            {
                                let headers = String::from_utf8_lossy(&request[..end]);
                                let length: usize = headers
                                    .lines()
                                    .find_map(|line| {
                                        line.to_lowercase()
                                            .strip_prefix("content-length:")
                                            .map(|value| value.trim().parse().unwrap())
                                    })
                                    .unwrap();
                                if request.len() >= end + 4 + length {
                                    break;
                                }
                            }
                        }
                        let generation = rotations.fetch_add(1, Ordering::SeqCst);
                        // Rotate once; reuse of an old refresh token is rejected.
                        let expected = format!("refresh_token=refresh-{generation}");
                        let valid = String::from_utf8_lossy(&request).contains(&expected);
                        tokio::time::sleep(Duration::from_millis(30)).await;
                        let body = if valid {
                            format!("access-{}", generation + 1)
                        } else {
                            "refresh token reused".into()
                        };
                        let status = if valid { "200 OK" } else { "400 Bad Request" };
                        let response = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        socket.write_all(response.as_bytes()).await.unwrap();
                    });
                }
            })
        };
        let flight = Arc::new(Flight::default());
        let stored = Arc::new(Mutex::new(("refresh-0".to_string(), String::new())));
        let client = reqwest::Client::new();
        // A second expired batch must re-read the stored rotated refresh token.
        for batch in 1..=2 {
            stored.lock().await.1.clear();
            let start = Arc::new(tokio::sync::Barrier::new(24));
            let mut tasks = Vec::new();
            for _ in 0..24 {
                let (flight, stored, client, endpoint, start) = (
                    flight.clone(),
                    stored.clone(),
                    client.clone(),
                    endpoint.clone(),
                    start.clone(),
                );
                tasks.push(tokio::spawn(async move {
                    start.wait().await;
                    flight
                        .run(|| async {
                            // Read and persist are separate, as with the real secret store:
                            // no store mutex may accidentally serialize network refreshes.
                            let (refresh, access) = stored.lock().await.clone();
                            if !access.is_empty() {
                                return Ok(access);
                            }
                            let response = client
                                .post(endpoint)
                                .form(&[("refresh_token", refresh)])
                                .send()
                                .await
                                .unwrap();
                            if !response.status().is_success() {
                                return Err(AppError::internal("refresh token reused"));
                            }
                            let access = response.text().await.unwrap();
                            *stored.lock().await = (format!("refresh-{batch}"), access.clone());
                            Ok(access)
                        })
                        .await
                }));
            }
            for task in tasks {
                assert_eq!(task.await.unwrap().unwrap(), format!("access-{batch}"));
            }
            assert_eq!(rotations.load(Ordering::SeqCst), batch);
            assert_eq!(stored.lock().await.0, format!("refresh-{batch}"));
        }
        server.abort();
    }
    #[tokio::test]
    async fn overlapping_failures_are_shared_and_a_later_request_can_retry() {
        let flight = Arc::new(Flight::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let (begun, seen) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel();
        let first = {
            let flight = flight.clone();
            let calls = calls.clone();
            tokio::spawn(async move {
                flight
                    .run(|| async {
                        calls.fetch_add(1, Ordering::SeqCst);
                        begun.send(()).unwrap();
                        released.await.unwrap();
                        Err(AppError::Internal("fake refresh rejected".into()))
                    })
                    .await
            })
        };
        seen.await.unwrap();
        let mut second = std::pin::pin!(flight.run(|| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok("unexpected".into())
        }));
        std::future::poll_fn(|cx| {
            assert!(second.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        release.send(()).unwrap();
        let second = second.await;
        assert!(first.await.unwrap().is_err());
        assert!(second.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            flight
                .run(|| async { Ok("reconnected".into()) })
                .await
                .unwrap(),
            "reconnected"
        );
    }
    #[tokio::test]
    async fn cancellation_releases_the_guard_and_other_stores_do_not_block() {
        let flight = Arc::new(Flight::default());
        let (begun, seen) = tokio::sync::oneshot::channel();
        let owner = {
            let flight = flight.clone();
            tokio::spawn(async move {
                flight
                    .run(|| async {
                        begun.send(()).unwrap();
                        std::future::pending::<Result<String, AppError>>().await
                    })
                    .await
            })
        };
        seen.await.unwrap();
        let independent = Flight::default();
        assert_eq!(
            independent
                .run(|| async { Ok("independent".into()) })
                .await
                .unwrap(),
            "independent"
        );
        owner.abort();
        assert!(owner.await.unwrap_err().is_cancelled());
        assert_eq!(
            flight
                .run(|| async { Ok("after cancellation".into()) })
                .await
                .unwrap(),
            "after cancellation"
        );
    }
    #[tokio::test(start_paused = true)]
    async fn waiting_is_bounded_and_does_not_cancel_the_owner() {
        let flight = Flight::default();
        let guard = flight.result.lock().await;
        let result = flight.run(|| async { Ok("must not run".into()) }).await;
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Timed out waiting"));
        drop(guard);
        assert_eq!(
            flight.run(|| async { Ok("retry".into()) }).await.unwrap(),
            "retry"
        );
    }
    #[tokio::test]
    async fn google_credential_stores_remain_independent() {
        let (begun, seen) = tokio::sync::oneshot::channel();
        let owner = tokio::spawn(async move {
            coordinated(Store::GoogleWorkspace, || async {
                begun.send(()).unwrap();
                std::future::pending::<Result<String, AppError>>().await
            })
            .await
        });
        seen.await.unwrap();
        assert_eq!(
            coordinated(Store::GoogleConnected, || async {
                Ok("separate account".into())
            })
            .await
            .unwrap(),
            "separate account"
        );
        owner.abort();
        assert!(owner.await.unwrap_err().is_cancelled());
    }
}
