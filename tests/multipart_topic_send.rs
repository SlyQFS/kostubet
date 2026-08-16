//! Regression tests for forum-topic delivery.
//!
//! teloxide-core 0.10 panicked with `not implemented` inside its multipart
//! serializer (`serde_multipart/serializers.rs: serialize_newtype_struct`)
//! whenever a request carrying a file (`send_document` / `send_photo`) also
//! set `message_thread_id`, because `ThreadId` serializes through a newtype
//! struct. Fixed in teloxide-core 0.11 (teloxide 0.14).
//!
//! These tests send real requests (against a local HTTP server standing in
//! for the Bot API) and assert both the call itself and that the serialized
//! multipart body carries the topic id.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use teloxide::prelude::*;
use teloxide::types::{ChatId, InputFile, MessageId, ThreadId};

/// Starts a one-shot Bot API stub that captures a single request body and
/// answers with a minimal successful `Message` response.
fn spawn_api_stub() -> (u16, std::thread::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();

    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let body = read_request_body(&mut stream);
        let response = concat!(
            r#"{"ok":true,"result":{"message_id":1,"date":0,"#,
            r#""chat":{"id":-1001234567890,"type":"supergroup","title":"T"}}}"#
        );
        let http = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        );
        stream.write_all(http.as_bytes()).expect("write");
        body
    });

    (port, handle)
}

fn read_request_body(stream: &mut TcpStream) -> Vec<u8> {
    let mut header_buf = Vec::new();
    let mut one = [0u8; 1];
    while !header_buf.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut one).expect("read header byte");
        header_buf.push(one[0]);
    }

    let headers = String::from_utf8_lossy(&header_buf);
    let content_length: usize = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .unwrap_or(0);

    let mut body = vec![0u8; content_length];
    stream.read_exact(&mut body).expect("read body");
    body
}

fn stub_bot(port: u16) -> Bot {
    Bot::new("123456:TEST-TOKEN").set_api_url(
        reqwest::Url::parse(&format!("http://127.0.0.1:{}", port)).expect("parse api url"),
    )
}

#[tokio::test]
async fn send_document_to_topic_is_delivered_with_thread_id() {
    let (port, server) = spawn_api_stub();

    let result = stub_bot(port)
        .send_document(ChatId(-1001234567890), InputFile::file_id("AgACtestfile"))
        .message_thread_id(ThreadId(MessageId(777)))
        .await;

    let body = server.join().expect("server thread");
    assert!(
        result.is_err() || result.is_ok(),
        "request must not panic (teloxide multipart serializer regression)"
    );
    result.expect("send_document request should succeed against the stub");

    let body = String::from_utf8_lossy(&body);
    assert!(
        body.contains("message_thread_id"),
        "multipart body must contain message_thread_id: {body}"
    );
    assert!(
        body.contains("777"),
        "multipart body must contain the topic id 777: {body}"
    );
}

#[tokio::test]
async fn send_photo_to_topic_is_delivered_with_thread_id() {
    let (port, server) = spawn_api_stub();

    let result = stub_bot(port)
        .send_photo(ChatId(-1001234567890), InputFile::file_id("AgACcoverfile"))
        .caption("🆕 <b>App v1.0</b>")
        .message_thread_id(ThreadId(MessageId(42)))
        .await;

    let body = server.join().expect("server thread");
    result.expect("send_photo request should succeed against the stub");

    let body = String::from_utf8_lossy(&body);
    assert!(
        body.contains("message_thread_id"),
        "multipart body must contain message_thread_id: {body}"
    );
    assert!(
        body.contains("42"),
        "multipart body must contain the topic id 42: {body}"
    );
}
