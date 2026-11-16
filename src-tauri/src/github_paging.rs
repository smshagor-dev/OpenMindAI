//! Collect GitHub REST lists without a silent first-page cutoff.
use crate::app_error::AppError;
use reqwest::{header::HeaderMap, Client};
use serde::de::DeserializeOwned;
use std::collections::HashSet;

pub async fn collect<T: DeserializeOwned>(
    client: &Client,
    endpoint: &str,
    headers: HeaderMap,
    identity: impl Fn(&T) -> i64,
) -> Result<Vec<T>, AppError> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let mut page = 1_u64;
    loop {
        // Construct requests from the original endpoint, never from an untrusted
        // Link URL that could forward the account token to another destination.
        let response = client
            .get(endpoint)
            .headers(headers.clone())
            .query(&[("per_page", 100), ("page", page)])
            .send()
            .await
            .map_err(|error| AppError::GithubApiError(error.to_string()))?;
        if !response.status().is_success() {
            return Err(AppError::GithubApiError(format!(
                "GitHub list page {page} failed (status {})",
                response.status()
            )));
        }
        let entries: Vec<T> = response
            .json()
            .await
            .map_err(|error| AppError::GithubApiError(error.to_string()))?;
        let count = entries.len();
        let before = result.len();
        for entry in entries {
            if seen.insert(identity(&entry)) {
                result.push(entry);
            }
        }
        if count < 100 {
            return Ok(result);
        }
        if result.len() == before {
            return Err(AppError::GithubApiError(
                "GitHub pagination made no progress; retry the listing".into(),
            ));
        }
        page = page
            .checked_add(1)
            .ok_or_else(|| AppError::GithubApiError("GitHub page number overflow".into()))?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Duration,
    };
    #[derive(Debug, serde::Deserialize)]
    struct Row {
        id: i64,
    }
    fn fixture(pages: Vec<(u16, String)>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!(
            "http://{}/items?sort=updated",
            listener.local_addr().unwrap()
        );
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in pages {
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < deadline, "missing page request");
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                // Accepted sockets may inherit O_NONBLOCK on macOS.
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    assert_eq!(socket.read(&mut byte).unwrap(), 1);
                    request.push(byte[0]);
                }
                requests.push(String::from_utf8(request).unwrap());
                write!(socket,"HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\nLink: <https://attacker.invalid/collect>; rel=\"next\"\r\n\r\n{body}",body.len()).unwrap();
            }
            requests
        });
        (endpoint, worker)
    }
    fn rows(start: i64, end: i64) -> String {
        serde_json::to_string(
            &(start..end)
                .map(|id| serde_json::json!({"id":id}))
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }
    fn headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            "authorization",
            "Bearer synthetic-pagination-token".parse().unwrap(),
        );
        h
    }
    #[test]
    fn accepted_socket_waits_for_delayed_headers() {
        let (url, worker) = fixture(vec![(200, "[]".into())]);
        let address = url
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        let mut socket = std::net::TcpStream::connect(address).unwrap();
        std::thread::sleep(Duration::from_millis(25));
        socket
            .write_all(b"GET /items HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        assert!(response.ends_with("[]"));
        assert_eq!(worker.join().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn retrieves_later_pages_with_filters_auth_and_no_duplicate_ids() {
        let (url, worker) = fixture(vec![
            (200, rows(0, 100)),
            (200, rows(99, 199)),
            (200, rows(199, 237)),
        ]);
        let result = collect::<Row>(&Client::new(), &url, headers(), |x| x.id)
            .await
            .unwrap();
        assert_eq!(result.len(), 237);
        assert_eq!(result.last().unwrap().id, 236);
        for (index, request) in worker.join().unwrap().iter().enumerate() {
            assert!(request.starts_with(&format!(
                "GET /items?sort=updated&per_page=100&page={} ",
                index + 1
            )));
            assert!(request
                .to_lowercase()
                .contains("authorization: bearer synthetic-pagination-token"));
        }
    }
    #[tokio::test]
    async fn full_last_page_probes_an_empty_successor() {
        let (url, worker) = fixture(vec![(200, rows(0, 100)), (200, "[]".into())]);
        assert_eq!(
            collect::<Row>(&Client::new(), &url, headers(), |x| x.id)
                .await
                .unwrap()
                .len(),
            100
        );
        assert_eq!(worker.join().unwrap().len(), 2);
    }
    #[tokio::test]
    async fn empty_listing_finishes_after_one_request() {
        let (url, worker) = fixture(vec![(200, "[]".into())]);
        assert!(collect::<Row>(&Client::new(), &url, headers(), |x| x.id)
            .await
            .unwrap()
            .is_empty());
        worker.join().unwrap();
    }
    #[tokio::test]
    async fn later_rate_limit_or_invalid_json_is_an_error_not_partial_success() {
        for (status, body) in [(403, "rate limited"), (200, "invalid json")] {
            let (url, worker) = fixture(vec![(200, rows(0, 100)), (status, body.into())]);
            assert!(collect::<Row>(&Client::new(), &url, headers(), |x| x.id)
                .await
                .is_err());
            worker.join().unwrap();
        }
    }
    #[tokio::test]
    async fn repeated_full_page_cannot_loop_forever() {
        let (url, worker) = fixture(vec![(200, rows(0, 100)), (200, rows(0, 100))]);
        assert!(collect::<Row>(&Client::new(), &url, headers(), |x| x.id)
            .await
            .unwrap_err()
            .to_string()
            .contains("no progress"));
        worker.join().unwrap();
    }
}
