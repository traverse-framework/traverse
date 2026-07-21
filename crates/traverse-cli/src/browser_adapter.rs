use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};
use traverse_runtime::{
    BrowserRuntimeSubscriptionErrorCode, BrowserRuntimeSubscriptionMessage,
    BrowserRuntimeSubscriptionRequest, RuntimeExecutionOutcome, browser_subscription_messages,
};

const ADAPTER_KIND: &str = "local_browser_subscription_created";
const ADAPTER_SCHEMA_VERSION: &str = "1.0.0";
const ADAPTER_GOVERNING_SPEC: &str = "019-local-browser-adapter-transport";
const SETUP_ERROR_KIND: &str = "local_browser_subscription_setup_error";
const STREAM_ERROR_KIND: &str = "local_browser_subscription_stream_error";
const LISTENING_PREFIX: &str = "local browser adapter listening on ";
const MAX_REQUEST_HEADER_BYTES: usize = 64 * 1024;
const MAX_REQUEST_BODY_BYTES: usize = 4 * 1024 * 1024;
const REQUEST_BODY_TOO_LARGE: &str = "browser adapter request body too large";
// This adapter serves one connection at a time by design (spec
// 019-local-browser-adapter-transport, local dev tool). Bounded socket
// timeouts still apply so a slow or idle caller cannot hang the process
// indefinitely (spec 033-http-json-api connection-handling model, issue
// #581); a bounded worker pool was not added here since this transport is
// not intended to serve concurrent production traffic.
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_DEADLINE: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
struct CreateSubscriptionRequest {
    subscription_request: BrowserRuntimeSubscriptionRequest,
}

#[derive(Debug)]
struct CreatedSubscription {
    messages: Vec<BrowserRuntimeSubscriptionMessage>,
}

#[derive(Debug)]
struct LocalBrowserAdapter {
    outcome: RuntimeExecutionOutcome,
    created_subscriptions: HashMap<String, CreatedSubscription>,
    next_subscription_index: u64,
}

pub fn serve_local_browser_adapter(bind_address: &str) -> Result<(), String> {
    let outcome = crate::canonical_expedition_runtime_outcome().map_err(|e| e.to_string())?;
    let listener = TcpListener::bind(bind_address).map_err(|error| {
        format!("failed to bind local browser adapter at {bind_address}: {error}")
    })?;
    let local_address = listener
        .local_addr()
        .map_err(|error| format!("failed to read local browser adapter address: {error}"))?;

    let mut adapter = LocalBrowserAdapter::new(outcome);
    println!("{LISTENING_PREFIX}http://{local_address}");
    let _ = std::io::stdout().flush();

    for connection in listener.incoming() {
        match connection {
            Ok(mut stream) => {
                if stream.set_read_timeout(Some(READ_TIMEOUT)).is_err()
                    || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err()
                {
                    eprintln!("local browser adapter: failed to configure connection timeouts");
                    continue;
                }
                if let Err(error) = adapter.handle_connection(&mut stream) {
                    let _ = write_plain_response(
                        &mut stream,
                        500,
                        "internal server error",
                        &json_error(SETUP_ERROR_KIND, "internal_server_error", &error),
                    );
                }
            }
            Err(error) => {
                return Err(format!("local browser adapter connection failed: {error}"));
            }
        }
    }

    Ok(())
}

impl LocalBrowserAdapter {
    fn new(outcome: RuntimeExecutionOutcome) -> Self {
        Self {
            outcome,
            created_subscriptions: HashMap::new(),
            next_subscription_index: 0,
        }
    }

    fn handle_connection(&mut self, stream: &mut TcpStream) -> Result<(), String> {
        let deadline = Instant::now() + REQUEST_DEADLINE;
        let request = match read_http_request(stream, deadline) {
            Ok(request) => request,
            Err(error) if error == REQUEST_BODY_TOO_LARGE => {
                return write_plain_response(
                    stream,
                    413,
                    "payload too large",
                    &json_error(
                        SETUP_ERROR_KIND,
                        "request_too_large",
                        "browser adapter request body exceeds the 4 MiB limit",
                    ),
                );
            }
            Err(error) => return Err(error),
        };
        match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/local/browser-subscriptions") => {
                self.handle_create_subscription(stream, &request)
            }
            ("GET", path) if path.starts_with("/local/browser-subscriptions/") => {
                self.handle_stream_request(stream, path, &request.headers)
            }
            _ => write_plain_response(
                stream,
                404,
                "not found",
                &json_error(
                    STREAM_ERROR_KIND,
                    "not_found",
                    "requested browser adapter route was not found",
                ),
            ),
        }
    }

    fn handle_create_subscription<W: Write>(
        &mut self,
        stream: &mut W,
        request: &HttpRequest,
    ) -> Result<(), String> {
        if !request
            .headers
            .get("content-type")
            .is_some_and(|value| value.contains("application/json"))
        {
            return write_plain_response(
                stream,
                400,
                "bad request",
                &json_error(
                    SETUP_ERROR_KIND,
                    "invalid_request",
                    "content-type must equal application/json",
                ),
            );
        }

        let payload = match serde_json::from_slice::<CreateSubscriptionRequest>(&request.body) {
            Ok(payload) => payload,
            Err(error) => {
                return write_plain_response(
                    stream,
                    400,
                    "bad request",
                    &json_error(
                        SETUP_ERROR_KIND,
                        "invalid_request",
                        &format!("failed to parse create-subscription request: {error}"),
                    ),
                );
            }
        };

        let messages = browser_subscription_messages(&payload.subscription_request, &self.outcome);

        if let Some(error) = messages.iter().find_map(|message| match message {
            BrowserRuntimeSubscriptionMessage::Error(error) => Some(error),
            _ => None,
        }) {
            let (status, code) = match error.code {
                BrowserRuntimeSubscriptionErrorCode::InvalidRequest => (400, "invalid_request"),
                BrowserRuntimeSubscriptionErrorCode::NotFound => (404, "not_found"),
                BrowserRuntimeSubscriptionErrorCode::UnsupportedOperation => {
                    (400, "unsupported_operation")
                }
            };
            let response = json_error(SETUP_ERROR_KIND, code, &error.message);
            return write_plain_response(stream, status, "error", &response);
        }

        let subscription_id = self.next_subscription_id();
        let request_id = self.outcome.result.request_id.clone();
        let execution_id = self.outcome.result.execution_id.clone();
        let stream_url = format!("/local/browser-subscriptions/{subscription_id}/stream");
        self.created_subscriptions
            .insert(subscription_id.clone(), CreatedSubscription { messages });

        write_json_response(
            stream,
            201,
            "created",
            &json!({
                "kind": ADAPTER_KIND,
                "schema_version": ADAPTER_SCHEMA_VERSION,
                "governing_spec": ADAPTER_GOVERNING_SPEC,
                "subscription_id": subscription_id,
                "stream_url": stream_url,
                "request_id": request_id,
                "execution_id": execution_id,
            }),
        )
    }

    fn handle_stream_request<W: Write>(
        &mut self,
        stream: &mut W,
        path: &str,
        headers: &HashMap<String, String>,
    ) -> Result<(), String> {
        if !headers
            .get("accept")
            .is_some_and(|value| value.contains("text/event-stream"))
        {
            return write_plain_response(
                stream,
                400,
                "bad request",
                &json_error(
                    STREAM_ERROR_KIND,
                    "invalid_request",
                    "accept must include text/event-stream",
                ),
            );
        }

        let Some(subscription_id) = path
            .strip_prefix("/local/browser-subscriptions/")
            .and_then(|tail| tail.strip_suffix("/stream"))
        else {
            return write_plain_response(
                stream,
                404,
                "not found",
                &json_error(
                    STREAM_ERROR_KIND,
                    "not_found",
                    "requested browser adapter stream was not found",
                ),
            );
        };

        let Some(created) = self.created_subscriptions.remove(subscription_id) else {
            return write_plain_response(
                stream,
                404,
                "not found",
                &json_error(
                    STREAM_ERROR_KIND,
                    "not_found",
                    &format!("subscription_id {subscription_id} was not found"),
                ),
            );
        };

        let mut body = String::new();
        for message in created.messages {
            let data = serde_json::to_string(&message).map_err(|error| {
                format!("failed to encode browser subscription message: {error}")
            })?;
            body.push_str("event: traverse_message\n");
            body.push_str("data: ");
            body.push_str(&data);
            body.push_str("\n\n");
        }

        write_response(stream, 200, "ok", "text/event-stream", body.as_bytes())
    }

    fn next_subscription_id(&mut self) -> String {
        self.next_subscription_index += 1;
        format!("lbs_{:04}", self.next_subscription_index)
    }
}

fn read_http_request(stream: &mut TcpStream, deadline: Instant) -> Result<HttpRequest, String> {
    let mut buffer = Vec::new();
    let mut header_end = None;
    loop {
        if Instant::now() >= deadline {
            return Err("browser adapter request timed out reading headers".to_string());
        }
        let mut chunk = [0_u8; 1024];
        let read = stream.read(&mut chunk).map_err(|error| {
            if is_timeout_error(&error) {
                "browser adapter request timed out reading headers".to_string()
            } else {
                format!("failed to read browser adapter request: {error}")
            }
        })?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        // Enforce the size cap before checking for the terminator so a
        // header block that completes in the same read that pushes it over
        // the cap is still rejected.
        if buffer.len() > MAX_REQUEST_HEADER_BYTES {
            return Err("browser adapter request too large".to_string());
        }
        if let Some(index) = find_header_end(&buffer) {
            header_end = Some(index);
            break;
        }
    }

    let Some(header_end) = header_end else {
        return Err("browser adapter request missing header terminator".to_string());
    };

    let headers_text = String::from_utf8(buffer[..header_end].to_vec()).map_err(|error| {
        format!("browser adapter request headers were not valid UTF-8: {error}")
    })?;
    let mut lines = headers_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| "browser adapter request missing request line".to_string())?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| "browser adapter request missing method".to_string())?
        .to_string();
    let path = request_parts
        .next()
        .ok_or_else(|| "browser adapter request missing path".to_string())?
        .to_string();

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_REQUEST_BODY_BYTES {
        return Err(REQUEST_BODY_TOO_LARGE.to_string());
    }
    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < content_length {
        if Instant::now() >= deadline {
            return Err("browser adapter request timed out reading body".to_string());
        }
        let mut chunk = [0_u8; 1024];
        let read = stream.read(&mut chunk).map_err(|error| {
            if is_timeout_error(&error) {
                "browser adapter request timed out reading body".to_string()
            } else {
                format!("failed to read browser adapter request body: {error}")
            }
        })?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);

    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn is_timeout_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn write_json_response<T: Serialize, W: Write>(
    stream: &mut W,
    status: u16,
    reason: &str,
    body: &T,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(body)
        .map_err(|error| format!("failed to serialize browser adapter response: {error}"))?;
    write_response(stream, status, reason, "application/json", &bytes)
}

fn write_plain_response<W: Write>(
    stream: &mut W,
    status: u16,
    reason: &str,
    body: &str,
) -> Result<(), String> {
    write_response(stream, status, reason, "application/json", body.as_bytes())
}

fn write_response<W: Write>(
    stream: &mut W,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| format!("failed to write browser adapter response: {error}"))?;
    stream
        .write_all(body)
        .map_err(|error| format!("failed to write browser adapter response: {error}"))?;
    stream
        .flush()
        .map_err(|error| format!("failed to write browser adapter response: {error}"))
}

fn json_error(kind: &str, code: &str, message: &str) -> String {
    json!({
        "kind": kind,
        "schema_version": ADAPTER_SCHEMA_VERSION,
        "governing_spec": ADAPTER_GOVERNING_SPEC,
        "code": code,
        "message": message
    })
    .to_string()
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use serde_json::json;

    #[test]
    fn create_subscription_returns_created_response_for_exact_request_id() {
        let mut adapter = LocalBrowserAdapter::new(canonical_outcome());
        let request = HttpRequest {
            method: "POST".to_string(),
            path: "/local/browser-subscriptions".to_string(),
            headers: HashMap::from([("content-type".to_string(), "application/json".to_string())]),
            body: json!({
                "subscription_request": {
                    "kind": "browser_runtime_subscription_request",
                    "schema_version": "1.0.0",
                    "governing_spec": "013-browser-runtime-subscription",
                    "request_id": "expedition-plan-request-001"
                }
            })
            .to_string()
            .into_bytes(),
        };
        let mut stream = TestStream::default();

        adapter
            .handle_create_subscription(&mut stream, &request)
            .expect("create should succeed");

        assert!(
            stream
                .response
                .contains("local_browser_subscription_created")
        );
        assert!(
            stream
                .response
                .contains("/local/browser-subscriptions/lbs_0001/stream")
        );
        assert!(stream.response.contains("expedition-plan-request-001"));
    }

    #[test]
    fn create_subscription_rejects_invalid_request_shape() {
        let mut adapter = LocalBrowserAdapter::new(canonical_outcome());
        let request = HttpRequest {
            method: "POST".to_string(),
            path: "/local/browser-subscriptions".to_string(),
            headers: HashMap::from([("content-type".to_string(), "application/json".to_string())]),
            body: br#"{"wrong":"shape"}"#.to_vec(),
        };
        let mut stream = TestStream::default();

        adapter
            .handle_create_subscription(&mut stream, &request)
            .expect("invalid request should still write a response");

        assert!(
            stream
                .response
                .contains("local_browser_subscription_setup_error")
        );
        assert!(stream.response.contains("invalid_request"));
    }

    #[test]
    fn stream_request_rejects_missing_subscription() {
        let mut adapter = LocalBrowserAdapter::new(canonical_outcome());
        let mut stream = TestStream::default();
        let headers = HashMap::from([("accept".to_string(), "text/event-stream".to_string())]);

        adapter
            .handle_stream_request(
                &mut stream,
                "/local/browser-subscriptions/lbs_0001/stream",
                &headers,
            )
            .expect("missing stream should still write a response");

        assert!(
            stream
                .response
                .contains("local_browser_subscription_stream_error")
        );
        assert!(stream.response.contains("not_found"));
    }

    fn canonical_outcome() -> RuntimeExecutionOutcome {
        crate::canonical_expedition_runtime_outcome().expect("canonical outcome should build")
    }

    #[test]
    fn create_subscription_reports_not_found_for_mismatched_request_id() {
        let mut adapter = LocalBrowserAdapter::new(canonical_outcome());
        let request = HttpRequest {
            method: "POST".to_string(),
            path: "/local/browser-subscriptions".to_string(),
            headers: HashMap::from([("content-type".to_string(), "application/json".to_string())]),
            body: json!({
                "subscription_request": {
                    "kind": "browser_runtime_subscription_request",
                    "schema_version": "1.0.0",
                    "governing_spec": "013-browser-runtime-subscription",
                    "request_id": "does-not-exist"
                }
            })
            .to_string()
            .into_bytes(),
        };
        let mut stream = TestStream::default();

        adapter
            .handle_create_subscription(&mut stream, &request)
            .expect("mismatched target should still write a response");

        assert!(stream.response.contains("not_found"));
    }

    #[test]
    fn create_subscription_reports_invalid_request_for_missing_target_selector() {
        let mut adapter = LocalBrowserAdapter::new(canonical_outcome());
        let request = HttpRequest {
            method: "POST".to_string(),
            path: "/local/browser-subscriptions".to_string(),
            headers: HashMap::from([("content-type".to_string(), "application/json".to_string())]),
            body: json!({
                "subscription_request": {
                    "kind": "browser_runtime_subscription_request",
                    "schema_version": "1.0.0",
                    "governing_spec": "013-browser-runtime-subscription"
                }
            })
            .to_string()
            .into_bytes(),
        };
        let mut stream = TestStream::default();

        adapter
            .handle_create_subscription(&mut stream, &request)
            .expect("missing target selector should still write a response");

        assert!(stream.response.contains("invalid_request"));
    }

    #[test]
    fn create_subscription_requires_json_content_type() {
        let mut adapter = LocalBrowserAdapter::new(canonical_outcome());
        let request = HttpRequest {
            method: "POST".to_string(),
            path: "/local/browser-subscriptions".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        };
        let mut stream = TestStream::default();

        adapter
            .handle_create_subscription(&mut stream, &request)
            .expect("missing content-type should still write a response");

        assert!(stream.response.contains("invalid_request"));
        assert!(stream.response.contains("content-type"));
    }

    #[test]
    fn stream_request_requires_event_stream_accept_header() {
        let mut adapter = LocalBrowserAdapter::new(canonical_outcome());
        let mut stream = TestStream::default();

        adapter
            .handle_stream_request(
                &mut stream,
                "/local/browser-subscriptions/lbs_0001/stream",
                &HashMap::new(),
            )
            .expect("missing accept header should still write a response");

        assert!(stream.response.contains("invalid_request"));
    }

    #[test]
    fn stream_request_rejects_path_without_stream_suffix() {
        let mut adapter = LocalBrowserAdapter::new(canonical_outcome());
        let mut stream = TestStream::default();
        let headers = HashMap::from([("accept".to_string(), "text/event-stream".to_string())]);

        adapter
            .handle_stream_request(
                &mut stream,
                "/local/browser-subscriptions/lbs_0001",
                &headers,
            )
            .expect("malformed stream path should still write a response");

        assert!(stream.response.contains("not_found"));
    }

    #[test]
    fn stream_request_returns_created_subscription_messages() {
        let mut adapter = LocalBrowserAdapter::new(canonical_outcome());
        let create_request = HttpRequest {
            method: "POST".to_string(),
            path: "/local/browser-subscriptions".to_string(),
            headers: HashMap::from([("content-type".to_string(), "application/json".to_string())]),
            body: json!({
                "subscription_request": {
                    "kind": "browser_runtime_subscription_request",
                    "schema_version": "1.0.0",
                    "governing_spec": "013-browser-runtime-subscription",
                    "request_id": "expedition-plan-request-001"
                }
            })
            .to_string()
            .into_bytes(),
        };
        let mut create_stream = TestStream::default();
        adapter
            .handle_create_subscription(&mut create_stream, &create_request)
            .expect("create should succeed");

        let mut stream_stream = TestStream::default();
        let headers = HashMap::from([("accept".to_string(), "text/event-stream".to_string())]);
        adapter
            .handle_stream_request(
                &mut stream_stream,
                "/local/browser-subscriptions/lbs_0001/stream",
                &headers,
            )
            .expect("stream should succeed");

        assert!(stream_stream.response.contains("text/event-stream"));
        assert!(stream_stream.response.contains("event: traverse_message"));
    }

    fn spawn_test_adapter() -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind must succeed");
        let address = listener.local_addr().expect("local addr must resolve");
        let outcome = canonical_outcome();
        let mut adapter = LocalBrowserAdapter::new(outcome);
        std::thread::spawn(move || {
            for connection in listener.incoming() {
                let Ok(mut stream) = connection else {
                    break;
                };
                if let Err(error) = adapter.handle_connection(&mut stream) {
                    let _ = write_plain_response(
                        &mut stream,
                        500,
                        "internal server error",
                        &json_error(SETUP_ERROR_KIND, "internal_server_error", &error),
                    );
                }
            }
        });
        address
    }

    fn read_all(stream: &mut TcpStream) -> Vec<u8> {
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("client read timeout must be set");
        let mut out = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&chunk[..n]),
            }
        }
        out
    }

    #[test]
    fn create_subscription_over_a_real_socket_returns_created_response() {
        let address = spawn_test_adapter();
        let mut client = TcpStream::connect(address).expect("client connection must connect");
        let body = json!({
            "subscription_request": {
                "kind": "browser_runtime_subscription_request",
                "schema_version": "1.0.0",
                "governing_spec": "013-browser-runtime-subscription",
                "request_id": "expedition-plan-request-001"
            }
        })
        .to_string();
        let request = format!(
            "POST /local/browser-subscriptions HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        client
            .write_all(request.as_bytes())
            .expect("request must write");
        let response = read_all(&mut client);
        let text = String::from_utf8_lossy(&response);
        assert!(text.contains("201 created"));
        assert!(text.contains("local_browser_subscription_created"));
    }

    #[test]
    fn unmatched_route_over_a_real_socket_returns_404() {
        let address = spawn_test_adapter();
        let mut client = TcpStream::connect(address).expect("client connection must connect");
        client
            .write_all(b"GET /nonexistent HTTP/1.1\r\nHost: x\r\n\r\n")
            .expect("request must write");
        let response = read_all(&mut client);
        let text = String::from_utf8_lossy(&response);
        assert!(text.contains("404 not found"));
    }

    #[test]
    fn stream_request_over_a_real_socket_returns_event_stream() {
        let address = spawn_test_adapter();

        let mut create_client =
            TcpStream::connect(address).expect("create connection must connect");
        let body = json!({
            "subscription_request": {
                "kind": "browser_runtime_subscription_request",
                "schema_version": "1.0.0",
                "governing_spec": "013-browser-runtime-subscription",
                "request_id": "expedition-plan-request-001"
            }
        })
        .to_string();
        let create_request = format!(
            "POST /local/browser-subscriptions HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        create_client
            .write_all(create_request.as_bytes())
            .expect("create request must write");
        let create_response = read_all(&mut create_client);
        assert!(String::from_utf8_lossy(&create_response).contains("201 created"));

        let mut stream_client =
            TcpStream::connect(address).expect("stream connection must connect");
        stream_client
            .write_all(
                b"GET /local/browser-subscriptions/lbs_0001/stream HTTP/1.1\r\nHost: x\r\nAccept: text/event-stream\r\n\r\n",
            )
            .expect("stream request must write");
        let stream_response = read_all(&mut stream_client);
        let text = String::from_utf8_lossy(&stream_response);
        assert!(text.contains("text/event-stream"));
    }

    #[test]
    fn oversized_headers_are_rejected_over_a_real_socket() {
        let address = spawn_test_adapter();
        let mut client = TcpStream::connect(address).expect("client connection must connect");
        // Large enough that the 64 KiB size cap is crossed many read chunks
        // (1 KiB each) before the terminator-bearing chunk is ever read, so
        // this doesn't depend on exactly where within a chunk the cap and
        // the terminator happen to land.
        let oversized_header_value = "x".repeat(4 * 64 * 1024);
        let request =
            format!("GET /x HTTP/1.1\r\nHost: x\r\nX-Filler: {oversized_header_value}\r\n\r\n");
        client
            .write_all(request.as_bytes())
            .expect("oversized header request must write");
        let response = read_all(&mut client);
        // read_http_request errors out of handle_connection before writing
        // anything; serve_local_browser_adapter's loop then writes a 500 for
        // any such error (it doesn't distinguish error kinds like http_api.rs
        // does).
        assert!(String::from_utf8_lossy(&response).contains("500"));
    }

    #[test]
    fn oversized_content_length_is_rejected_before_body_allocation() {
        let address = spawn_test_adapter();
        let mut client = TcpStream::connect(address).expect("client connection must connect");
        let request = format!(
            "POST /local/browser-subscriptions HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            MAX_REQUEST_BODY_BYTES + 1
        );
        client
            .write_all(request.as_bytes())
            .expect("oversized request headers must write");
        let response = read_all(&mut client);
        let response = String::from_utf8_lossy(&response);

        assert!(response.contains("413 payload too large"));
        assert!(response.contains("request_too_large"));
    }

    #[test]
    fn missing_header_terminator_is_rejected_over_a_real_socket() {
        let address = spawn_test_adapter();
        let mut client = TcpStream::connect(address).expect("client connection must connect");
        client
            .write_all(b"GET /x HTTP/1.1\r\nHost: x\r\n")
            .expect("partial request must write");
        drop(client.shutdown(std::net::Shutdown::Write));
        let response = read_all(&mut client);
        assert!(String::from_utf8_lossy(&response).contains("500"));
    }

    #[test]
    fn invalid_utf8_headers_are_rejected_over_a_real_socket() {
        let address = spawn_test_adapter();
        let mut client = TcpStream::connect(address).expect("client connection must connect");
        let mut request = b"GET /x HTTP/1.1\r\nHost: ".to_vec();
        request.extend_from_slice(&[0xFF, 0xFE]);
        request.extend_from_slice(b"\r\n\r\n");
        client
            .write_all(&request)
            .expect("invalid utf8 request must write");
        let response = read_all(&mut client);
        assert!(String::from_utf8_lossy(&response).contains("500"));
    }

    #[test]
    fn body_shorter_than_content_length_stops_at_eof_over_a_real_socket() {
        let address = spawn_test_adapter();
        let mut client = TcpStream::connect(address).expect("client connection must connect");
        client
            .write_all(
                b"POST /local/browser-subscriptions HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 1000\r\n\r\nshort",
            )
            .expect("request must write");
        drop(client.shutdown(std::net::Shutdown::Write));
        let response = read_all(&mut client);
        // The body never reaches content_length before EOF; read_http_request
        // truncates to what arrived and the (invalid) JSON is then rejected
        // as a bad request, not a 500.
        assert!(String::from_utf8_lossy(&response).contains("400"));
    }

    #[test]
    fn request_with_body_beyond_header_terminator_is_read_over_a_real_socket() {
        let address = spawn_test_adapter();
        let mut client = TcpStream::connect(address).expect("client connection must connect");
        let body = json!({
            "subscription_request": {
                "kind": "browser_runtime_subscription_request",
                "schema_version": "1.0.0",
                "governing_spec": "013-browser-runtime-subscription",
                "request_id": "expedition-plan-request-001"
            }
        })
        .to_string();
        // Send headers and body in two separate writes so read_http_request
        // must loop to read the remaining content-length bytes.
        let headers = format!(
            "POST /local/browser-subscriptions HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        client
            .write_all(headers.as_bytes())
            .expect("headers must write");
        std::thread::sleep(std::time::Duration::from_millis(50));
        client.write_all(body.as_bytes()).expect("body must write");
        let response = read_all(&mut client);
        assert!(String::from_utf8_lossy(&response).contains("201 created"));
    }

    #[derive(Default)]
    struct TestStream {
        response: String,
    }

    impl Write for TestStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.response.push_str(&String::from_utf8_lossy(buf));
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
