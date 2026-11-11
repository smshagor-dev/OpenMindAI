//! Bounded JSON-RPC response selection for JSON and streaming SSE transports.
use std::time::Duration;

use reqwest::{header, Response};
use serde_json::Value;

fn result_for(value: Value, expected_id: &Value) -> Result<Option<Value>, String> {
    if value.get("id") != Some(expected_id) {
        return Ok(None);
    }
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err("invalid JSON-RPC version in matching response".into());
    }
    match (value.get("result"), value.get("error")) {
        (Some(result), None) => Ok(Some(result.clone())),
        (None, Some(error)) => Err(format!("JSON-RPC error: {error}")),
        _ => Err("matching JSON-RPC response must contain exactly one of result or error".into()),
    }
}

#[derive(Default)]
struct Events {
    line: Vec<u8>,
    data: String,
    first_line: bool,
    after_cr: bool,
}

impl Events {
    fn new() -> Self {
        Self {
            first_line: true,
            ..Self::default()
        }
    }

    fn finish_line(&mut self, expected_id: &Value) -> Result<Option<Value>, String> {
        let bytes = std::mem::take(&mut self.line);
        let line = std::str::from_utf8(&bytes).map_err(|_| "invalid UTF-8 in SSE response")?;
        let line = if std::mem::take(&mut self.first_line) {
            line.strip_prefix('\u{feff}').unwrap_or(line)
        } else {
            line
        };
        if line.is_empty() {
            if self.data.is_empty() {
                return Ok(None);
            }
            // SSE joins data fields with LF and removes the last LF on dispatch.
            self.data.pop();
            let value = serde_json::from_str(&self.data)
                .map_err(|error| format!("invalid SSE JSON response: {error}"))?;
            self.data.clear();
            return result_for(value, expected_id);
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        if field == "data" {
            self.data.push_str(value.strip_prefix(' ').unwrap_or(value));
            self.data.push('\n');
        }
        Ok(None)
    }

    fn push(&mut self, chunk: &[u8], expected_id: &Value) -> Result<Option<Value>, String> {
        for &byte in chunk {
            if self.after_cr && byte == b'\n' {
                self.after_cr = false;
                continue;
            }
            self.after_cr = byte == b'\r';
            if byte == b'\r' || byte == b'\n' {
                if let Some(result) = self.finish_line(expected_id)? {
                    return Ok(Some(result));
                }
            } else {
                self.line.push(byte);
            }
        }
        Ok(None)
    }
}

pub async fn read_response(
    response: Response,
    expected_id: &Value,
    max_bytes: usize,
    deadline: Duration,
) -> Result<Value, String> {
    tokio::time::timeout(deadline, read_inner(response, expected_id, max_bytes))
        .await
        .map_err(|_| "MCP response timed out".to_owned())?
}

async fn read_inner(
    mut response: Response,
    expected_id: &Value,
    max_bytes: usize,
) -> Result<Value, String> {
    if !response.status().is_success() {
        return Err(format!("remote API returned status {}", response.status()));
    }
    let is_sse = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"));
    let mut total = 0_usize;
    let mut body = Vec::new();
    let mut events = Events::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("MCP response read failed: {error}"))?
    {
        total = total.saturating_add(chunk.len());
        if total > max_bytes {
            return Err(format!(
                "response exceeded the {max_bytes} byte safety limit"
            ));
        }
        if is_sse {
            if let Some(result) = events.push(&chunk, expected_id)? {
                return Ok(result);
            }
        } else {
            body.extend_from_slice(&chunk);
        }
    }
    if is_sse {
        // An event without its terminating blank line is incomplete, even at EOF.
        return Err("SSE stream ended without a complete matching JSON-RPC response".into());
    }
    let value = serde_json::from_slice(&body)
        .map_err(|error| format!("invalid JSON-RPC response: {error}"))?;
    result_for(value, expected_id)?
        .ok_or_else(|| "JSON-RPC response ID does not match request".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        task::JoinHandle,
    };

    struct Server(JoinHandle<()>);
    impl Drop for Server {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    async fn fixture(
        content_type: &str,
        status: &str,
        pieces: Vec<Vec<u8>>,
        hold_open: bool,
    ) -> (Response, Server) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let headers = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\n\r\n");
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_request_head(&mut socket).await;
            socket.write_all(headers.as_bytes()).await.unwrap();
            for piece in pieces {
                socket
                    .write_all(format!("{:x}\r\n", piece.len()).as_bytes())
                    .await
                    .unwrap();
                socket.write_all(&piece).await.unwrap();
                socket.write_all(b"\r\n").await.unwrap();
            }
            if hold_open {
                std::future::pending::<()>().await;
            }
            socket.write_all(b"0\r\n\r\n").await.unwrap();
        });
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/mcp"))
            .send()
            .await
            .unwrap();
        (response, Server(task))
    }

    async fn read_request_head(socket: &mut TcpStream) {
        let mut head = Vec::new();
        for _ in 0..4096 {
            let mut byte = [0];
            socket.read_exact(&mut byte).await.unwrap();
            head.push(byte[0]);
            if head.ends_with(b"\r\n\r\n") {
                return;
            }
        }
        panic!("fixture request header exceeded bound");
    }

    const RESPONSE: &str = "{\"jsonrpc\":\"2.0\",\"id\":\"req\",\"result\":{\"text\":\"বাংলা\"}}";

    #[test]
    fn frames_multiline_data_and_ignores_notifications_at_every_byte_boundary() {
        for newline in ["\n", "\r\n", "\r"] {
            let text = format!("\u{feff}: heartbeat{newline}event: message{newline}data: {{\"jsonrpc\":\"2.0\",\"method\":\"notice\"}}{newline}{newline}data: {{\"jsonrpc\":\"2.0\",{newline}data: \"id\":\"req\",\"result\":{{\"text\":\"বাংলা\"}}}}{newline}{newline}data: invalid later event{newline}{newline}");
            for split in 0..=text.len() {
                let mut events = Events::new();
                let first = events
                    .push(&text.as_bytes()[..split], &json!("req"))
                    .unwrap();
                let result = first.or_else(|| {
                    events
                        .push(&text.as_bytes()[split..], &json!("req"))
                        .unwrap()
                });
                assert_eq!(result, Some(json!({"text":"বাংলা"})), "split={split}");
            }
        }
    }

    #[test]
    fn id_types_versions_and_response_shapes_are_checked() {
        assert!(
            result_for(json!({"jsonrpc":"2.0","id":1,"result":1}), &json!("1"))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            result_for(json!({"jsonrpc":"2.0","id":1,"result":null}), &json!(1)).unwrap(),
            Some(Value::Null)
        );
        for value in [
            json!({"id":"req","result":1}),
            json!({"jsonrpc":"2.0","id":"req"}),
            json!({"jsonrpc":"2.0","id":"req","result":1,"error":{}}),
        ] {
            assert!(result_for(value, &json!("req")).is_err());
        }
    }

    #[tokio::test]
    async fn streaming_match_returns_before_server_closes_or_sends_later_events() {
        let body = format!("data: {{\"jsonrpc\":\"2.0\",\"id\":\"other\",\"result\":false}}\n\ndata: {RESPONSE}\n\ndata: {{\"method\":\"notification\"}}\n\n");
        let pieces = body.as_bytes().chunks(3).map(<[u8]>::to_vec).collect();
        let (response, _server) =
            fixture("Text/Event-Stream; charset=utf-8", "200 OK", pieces, true).await;
        assert_eq!(
            read_response(response, &json!("req"), 4096, Duration::from_secs(2))
                .await
                .unwrap(),
            json!({"text":"বাংলা"})
        );
    }

    #[tokio::test]
    async fn json_transport_correlates_id_and_preserves_result() {
        let (response, _server) = fixture(
            "application/json",
            "200 OK",
            vec![RESPONSE.as_bytes().to_vec()],
            false,
        )
        .await;
        assert_eq!(
            read_response(response, &json!("req"), 4096, Duration::from_secs(2))
                .await
                .unwrap(),
            json!({"text":"বাংলা"})
        );
        let (response, _server) = fixture(
            "application/json",
            "200 OK",
            vec![RESPONSE.as_bytes().to_vec()],
            false,
        )
        .await;
        assert!(
            read_response(response, &json!("different"), 4096, Duration::from_secs(2))
                .await
                .unwrap_err()
                .contains("ID does not match")
        );
    }

    #[tokio::test]
    async fn remote_rpc_errors_propagate_on_both_transports() {
        for content_type in ["application/json", "text/event-stream"] {
            let error = "{\"jsonrpc\":\"2.0\",\"id\":\"req\",\"error\":{\"code\":-32000,\"message\":\"synthetic failure\"}}";
            let body = if content_type == "text/event-stream" {
                format!("data: {error}\n\n")
            } else {
                error.into()
            };
            let (response, _server) =
                fixture(content_type, "200 OK", vec![body.into_bytes()], false).await;
            assert!(
                read_response(response, &json!("req"), 4096, Duration::from_secs(2))
                    .await
                    .unwrap_err()
                    .contains("synthetic failure")
            );
        }
    }

    #[tokio::test]
    async fn malformed_and_truncated_events_fail_without_selecting_unrelated_data() {
        for body in [
            "data: not json\n\n",
            "data: {\"id\":\"other\",\"result\":1}\n\n",
            "data: {}\n",
        ]
        .map(|s| s.as_bytes().to_vec())
        {
            let (response, _server) =
                fixture("text/event-stream", "200 OK", vec![body], false).await;
            assert!(
                read_response(response, &json!("req"), 4096, Duration::from_secs(2))
                    .await
                    .is_err()
            );
        }
        let (response, _server) = fixture(
            "text/event-stream",
            "200 OK",
            vec![b"data: \xff\n\n".to_vec()],
            false,
        )
        .await;
        assert!(
            read_response(response, &json!("req"), 4096, Duration::from_secs(2))
                .await
                .unwrap_err()
                .contains("UTF-8")
        );
    }

    #[tokio::test]
    async fn broken_http_body_is_reported_as_read_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_request_head(&mut socket).await;
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 999\r\n\r\ndata: {").await.unwrap();
        });
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}"))
            .send()
            .await
            .unwrap();
        assert!(
            read_response(response, &json!("req"), 4096, Duration::from_secs(2))
                .await
                .unwrap_err()
                .contains("read failed")
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn total_bytes_are_bounded_even_for_heartbeats_or_unterminated_lines() {
        for content_type in ["application/json", "text/event-stream"] {
            for body in [vec![b'x'; 128], b": heartbeat\n\n".repeat(20)] {
                let (response, _server) = fixture(content_type, "200 OK", vec![body], false).await;
                assert!(
                    read_response(response, &json!("req"), 64, Duration::from_secs(2))
                        .await
                        .unwrap_err()
                        .contains("safety limit")
                );
            }
        }
    }

    #[tokio::test]
    async fn missing_match_times_out_and_http_failure_cannot_supply_a_result() {
        let (response, _server) = fixture(
            "text/event-stream",
            "200 OK",
            vec![b": heartbeat\n\n".to_vec()],
            true,
        )
        .await;
        assert!(
            read_response(response, &json!("req"), 4096, Duration::from_millis(50))
                .await
                .unwrap_err()
                .contains("timed out")
        );
        let (response, _server) = fixture(
            "text/event-stream",
            "500 Server Error",
            vec![format!("data: {RESPONSE}\n\n").into_bytes()],
            false,
        )
        .await;
        assert!(
            read_response(response, &json!("req"), 4096, Duration::from_secs(2))
                .await
                .unwrap_err()
                .contains("status 500")
        );
    }
}
