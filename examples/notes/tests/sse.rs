use std::{io, net::SocketAddr, time::Duration};

use notes_example::{AppState, http_router};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

async fn get_sse(address: SocketAddr) -> io::Result<Vec<u8>> {
    let mut stream = TcpStream::connect(address).await?;
    let request = format!(
        "GET /events/stream HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;

    let mut response = Vec::new();
    stream
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut response)
        .await?;
    if response.len() > MAX_RESPONSE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP response exceeded the configured size limit",
        ));
    }
    Ok(response)
}

fn decode_chunked_body(encoded: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut decoded = Vec::new();
    let mut cursor = 0;

    loop {
        let line_end = encoded
            .get(cursor..)
            .and_then(|remaining| remaining.windows(2).position(|window| window == b"\r\n"))
            .map(|offset| cursor + offset)
            .ok_or("missing chunk-size line terminator")?;
        let size_line = std::str::from_utf8(&encoded[cursor..line_end])
            .map_err(|_| "chunk-size line is not ASCII")?;
        let size_token = size_line.split(';').next().ok_or("missing chunk size")?;
        let size = usize::from_str_radix(size_token, 16).map_err(|_| "invalid chunk size")?;
        cursor = line_end + 2;

        if size == 0 {
            loop {
                let trailer_end = encoded
                    .get(cursor..)
                    .and_then(|remaining| remaining.windows(2).position(|window| window == b"\r\n"))
                    .map(|offset| cursor + offset)
                    .ok_or("missing chunk trailer terminator")?;
                let trailer = &encoded[cursor..trailer_end];
                cursor = trailer_end + 2;
                if trailer.is_empty() {
                    break;
                }
            }
            if cursor != encoded.len() {
                return Err("unexpected bytes after terminal chunk");
            }
            return Ok(decoded);
        }

        let data_end = cursor.checked_add(size).ok_or("chunk size overflow")?;
        let framed_end = data_end.checked_add(2).ok_or("chunk framing overflow")?;
        if framed_end > encoded.len() {
            return Err("truncated chunk data");
        }
        decoded.extend_from_slice(&encoded[cursor..data_end]);
        if &encoded[data_end..framed_end] != b"\r\n" {
            return Err("missing chunk-data terminator");
        }
        cursor = framed_end;
    }
}

#[tokio::test]
async fn generated_sse_hook_streams_fixture_events_over_loopback_and_terminates() {
    assert_eq!(
        notes_example::generated::GENERATED_SSE_ROUTES,
        &[notes_example::generated::GeneratedSseRoute {
            name: "subscribe_events",
            method: "GET",
            path: "/events/stream",
        }]
    );
    assert!(
        !notes_example::generated::GENERATED_ROUTES
            .iter()
            .any(|route| route.name == "subscribe_events"),
        "SSE must not become a generated unary route"
    );
    let generated_mcp: serde_json::Value =
        serde_json::from_str(notes_example::GENERATED_MCP_JSON).unwrap();
    assert!(
        !generated_mcp["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "subscribe_events"),
        "HTTP-only SSE must not be projected into MCP"
    );

    let app = http_router(AppState::with_fixtures());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown, shutdown_signal) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_signal.await;
            })
            .await
    });

    // Keep cleanup deterministic even if the bounded client request fails.
    let response = timeout(REQUEST_TIMEOUT, get_sse(address)).await;
    let _ = shutdown.send(());
    server.await.unwrap().unwrap();

    // read_to_end observes the connection EOF; the chunk decoder independently
    // requires the HTTP terminal chunk, so both transport and SSE completion are
    // exercised rather than inferred from an in-process router response.
    let response = response
        .expect("loopback HTTP request exceeded its timeout")
        .unwrap();
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP response header terminator");
    let headers = std::str::from_utf8(&response[..header_end]).unwrap();
    let encoded_body = &response[header_end + 4..];

    assert!(
        headers.lines().next().unwrap().starts_with("HTTP/1.1 200 "),
        "unexpected status line: {}",
        headers.lines().next().unwrap()
    );
    let content_type = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-type")
                .then_some(value.trim())
        })
        .expect("SSE Content-Type header");
    assert_eq!(content_type.split(';').next(), Some("text/event-stream"));
    assert!(headers.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("transfer-encoding")
                && value.to_ascii_lowercase().contains("chunked")
        })
    }));

    let body = decode_chunked_body(encoded_body).expect("complete chunked HTTP response");
    assert_eq!(
        body,
        b"event: note.created\ndata: {\"id\":\"n1\"}\n\nevent: note.updated\ndata: {\"id\":\"n1\",\"title\":\"fixture\"}\n\n"
    );
    println!(
        "GET /events/stream => HTTP 200; text/event-stream; 2 exact fixture events; terminal chunk and connection EOF observed"
    );
}
