//! Shared loopback OAuth callback receiver. Invalid connections never complete login.
use std::time::Duration;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinSet,
    time::timeout,
};
use url::Url;

const MAX_HEAD_BYTES: usize = 16 * 1024;
const MAX_CONNECTIONS: usize = 8;
const MAX_ATTEMPTS: usize = 128;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);

enum Callback {
    Code(String),
    Denied(String),
}

fn parse(head: &[u8], path: &str, expected_state: &str) -> Option<Callback> {
    let request = std::str::from_utf8(head).ok()?;
    let mut fields = request.split("\r\n").next()?.split_whitespace();
    if fields.next()? != "GET" {
        return None;
    }
    let target = fields.next()?;
    if !matches!(fields.next()?, "HTTP/1.0" | "HTTP/1.1")
        || fields.next().is_some()
        || !target.starts_with('/')
        || target.starts_with("//")
    {
        return None;
    }
    let callback = Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    if callback.path() != path {
        return None;
    }
    let (mut state, mut code, mut error) = (None, None, None);
    for (key, value) in callback.query_pairs() {
        let slot = match key.as_ref() {
            "state" => &mut state,
            "code" => &mut code,
            "error" => &mut error,
            _ => continue,
        };
        if slot.is_some() || value.is_empty() {
            return None;
        }
        *slot = Some(value.into_owned());
    }
    if state.as_deref() != Some(expected_state) {
        return None;
    }
    match (code, error) {
        (Some(code), None) => Some(Callback::Code(code)),
        (None, Some(error)) => Some(Callback::Denied(error)),
        _ => None,
    }
}

async fn read_head(socket: &mut TcpStream) -> Option<Vec<u8>> {
    let mut head = Vec::new();
    let mut chunk = [0; 1024];
    while head.len() < MAX_HEAD_BYTES {
        let limit = chunk.len().min(MAX_HEAD_BYTES - head.len());
        let count = socket.read(&mut chunk[..limit]).await.ok()?;
        if count == 0 {
            return None;
        }
        head.extend_from_slice(&chunk[..count]);
        if let Some(end) = head.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            head.truncate(end + 4);
            return Some(head);
        }
    }
    None
}

async fn connection(
    mut socket: TcpStream,
    path: String,
    expected_state: String,
) -> Option<Callback> {
    let outcome = match timeout(CONNECTION_TIMEOUT, read_head(&mut socket)).await {
        Ok(Some(head)) => parse(&head, &path, &expected_state),
        _ => None,
    };
    let (status, message) = match outcome {
        Some(Callback::Code(_)) => ("200 OK", "Account connected. You can return to OpenMindAI."),
        Some(Callback::Denied(_)) => (
            "200 OK",
            "Connection declined. Return to OpenMindAI for details.",
        ),
        None => (
            "400 Bad Request",
            "This request did not complete login. Return to the authorization page.",
        ),
    };
    // Browser disconnects must not discard an already authenticated callback.
    let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{message}", message.len());
    let _ = timeout(
        Duration::from_millis(500),
        socket.write_all(response.as_bytes()),
    )
    .await;
    outcome
}

pub async fn receive(
    listener: TcpListener,
    path: &str,
    expected_state: &str,
    deadline: Duration,
) -> Result<String, String> {
    if !listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .ip()
        .is_loopback()
    {
        return Err("OAuth callback listener must be loopback-only".into());
    }
    let work = async {
        let mut connections = JoinSet::new();
        let mut attempts = 0;
        loop {
            if attempts == MAX_ATTEMPTS && connections.is_empty() {
                return Err("OAuth callback connection limit exceeded".into());
            }
            tokio::select! {
                accepted = listener.accept(), if attempts < MAX_ATTEMPTS && connections.len() < MAX_CONNECTIONS => {
                    let (socket, peer) = accepted.map_err(|error| error.to_string())?;
                    attempts += 1;
                    if peer.ip().is_loopback() {
                        connections.spawn(connection(socket, path.to_owned(), expected_state.to_owned()));
                    }
                }
                finished = connections.join_next(), if !connections.is_empty() => {
                    match finished {
                        Some(Ok(Some(Callback::Code(code)))) => return Ok(code),
                        Some(Ok(Some(Callback::Denied(error)))) => return Err(format!("authorization returned '{error}'")),
                        _ => {}
                    }
                }
            }
        }
    };
    // Dropping this future also drops JoinSet and aborts every pending socket task.
    timeout(deadline, work)
        .await
        .map_err(|_| "authorization timed out".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    const PATH: &str = "/oauth2/callback";
    const STATE: &str = "synthetic-state";

    fn request(target: &str) -> String {
        format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n")
    }

    async fn start(
        address: &str,
        deadline: Duration,
    ) -> (SocketAddr, tokio::task::JoinHandle<Result<String, String>>) {
        let listener = TcpListener::bind(address).await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(receive(listener, PATH, STATE, deadline));
        (address, task)
    }

    async fn exchange(address: SocketAddr, bytes: &[u8]) -> String {
        let mut socket = TcpStream::connect(address).await.unwrap();
        socket.write_all(bytes).await.unwrap();
        let mut response = String::new();
        timeout(Duration::from_secs(2), socket.read_to_string(&mut response))
            .await
            .unwrap()
            .unwrap();
        response
    }

    #[tokio::test]
    async fn fragmented_callback_survives_unrelated_wrong_state_and_stalled_connections() {
        let (address, task) = start("127.0.0.1:0", Duration::from_secs(3)).await;
        let mut stalled = TcpStream::connect(address).await.unwrap();
        stalled.write_all(b"GET /incomplete").await.unwrap();
        for target in [
            "/favicon.ico",
            "/oauth2/callback?state=wrong&code=no",
            "/other?state=synthetic-state&code=no",
            "/oauth2/callback?state=wrong&error=access_denied",
        ] {
            assert!(exchange(address, request(target).as_bytes())
                .await
                .starts_with("HTTP/1.1 400"));
            assert!(!task.is_finished());
        }
        let mut socket = TcpStream::connect(address).await.unwrap();
        socket.write_all(b"GET /oauth2/callback?sta").await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        socket
            .write_all(b"te=synthetic-state&code=fragmented%2Bcode HTTP/1.1\r\nHost: localhost\r\n")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!task.is_finished());
        socket.write_all(b"\r\n").await.unwrap();
        assert_eq!(task.await.unwrap().unwrap(), "fragmented+code");
        let mut bytes = Vec::new();
        timeout(Duration::from_secs(1), stalled.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn only_exact_path_state_and_unambiguous_get_callbacks_are_accepted() {
        for target in [
            "http://localhost/oauth2/callback?state=synthetic-state&code=x",
            "//localhost/oauth2/callback?state=synthetic-state&code=x",
            "/oauth2/callback?state=synthetic-state&state=wrong&code=x",
            "/oauth2/callback?state=synthetic-state&code=x&code=y",
            "/oauth2/callback?state=synthetic-state&code=",
            "/oauth2/callback?state=synthetic-state&code=x&error=denied",
            "/oauth2/callback?code=x",
        ] {
            assert!(
                parse(request(target).as_bytes(), PATH, STATE).is_none(),
                "{target}"
            );
        }
        let head = request("/oauth2/callback?state=synthetic-state&code=x");
        assert!(parse(head.replace("GET", "POST").as_bytes(), PATH, STATE).is_none());
        assert!(
            matches!(parse(head.as_bytes(), PATH, STATE), Some(Callback::Code(code)) if code == "x")
        );
    }

    #[tokio::test]
    async fn oversized_and_truncated_requests_do_not_complete_login() {
        let (address, task) = start("127.0.0.1:0", Duration::from_secs(3)).await;
        assert!(exchange(address, &vec![b'x'; MAX_HEAD_BYTES])
            .await
            .starts_with("HTTP/1.1 400"));
        let mut socket = TcpStream::connect(address).await.unwrap();
        socket
            .write_all(b"GET /oauth2/callback?state=synthetic-state&code=partial HTTP/1.1\r\n")
            .await
            .unwrap();
        socket.shutdown().await.unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).await.unwrap();
        assert!(response.starts_with("HTTP/1.1 400"));
        assert!(exchange(
            address,
            request("/oauth2/callback?state=synthetic-state&code=complete").as_bytes()
        )
        .await
        .starts_with("HTTP/1.1 200"));
        assert_eq!(task.await.unwrap().unwrap(), "complete");
    }

    #[tokio::test]
    async fn correctly_bound_provider_error_is_terminal() {
        let (address, task) = start("127.0.0.1:0", Duration::from_secs(3)).await;
        assert!(exchange(
            address,
            request("/oauth2/callback?state=synthetic-state&error=access_denied").as_bytes()
        )
        .await
        .starts_with("HTTP/1.1 200"));
        assert!(task.await.unwrap().unwrap_err().contains("access_denied"));
    }

    #[tokio::test]
    async fn stalled_connection_has_its_own_deadline_and_login_can_continue() {
        let (address, task) = start("127.0.0.1:0", Duration::from_secs(9)).await;
        let mut stalled = TcpStream::connect(address).await.unwrap();
        stalled.write_all(b"GET /unfinished").await.unwrap();
        let mut response = String::new();
        timeout(
            Duration::from_secs(7),
            stalled.read_to_string(&mut response),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(response.starts_with("HTTP/1.1 400"));
        assert!(!task.is_finished());
        exchange(
            address,
            request("/oauth2/callback?state=synthetic-state&code=after-timeout").as_bytes(),
        )
        .await;
        assert_eq!(task.await.unwrap().unwrap(), "after-timeout");
    }

    #[tokio::test]
    async fn overall_timeout_and_cancellation_close_pending_sockets() {
        for cancel in [false, true] {
            let (address, task) = start("127.0.0.1:0", Duration::from_millis(50)).await;
            let mut socket = TcpStream::connect(address).await.unwrap();
            socket.write_all(b"GET /").await.unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                assert!(task.await.unwrap().unwrap_err().contains("timed out"));
            }
            let mut response = Vec::new();
            timeout(Duration::from_secs(1), socket.read_to_end(&mut response))
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn attempts_are_bounded_and_never_accept_invalid_state() {
        let (address, task) = start("127.0.0.1:0", Duration::from_secs(5)).await;
        for _ in 0..MAX_ATTEMPTS {
            assert!(exchange(
                address,
                request("/oauth2/callback?state=wrong&code=no").as_bytes()
            )
            .await
            .starts_with("HTTP/1.1 400"));
        }
        assert!(task
            .await
            .unwrap()
            .unwrap_err()
            .contains("connection limit"));
    }

    #[tokio::test]
    async fn ipv6_loopback_callback_works() {
        let (address, task) = start("[::1]:0", Duration::from_secs(3)).await;
        exchange(
            address,
            request("/oauth2/callback?state=synthetic-state&code=ipv6").as_bytes(),
        )
        .await;
        assert_eq!(task.await.unwrap().unwrap(), "ipv6");
    }
}
