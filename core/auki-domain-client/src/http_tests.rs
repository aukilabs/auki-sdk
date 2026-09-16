use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn raw_response(
    response: &'static [u8],
) -> (reqwest::RequestBuilder, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut chunk = [0; 1024];
            let read = socket.read(&mut chunk).await.unwrap();
            assert!(read > 0);
            request.extend_from_slice(&chunk[..read]);
            assert!(request.len() <= 4096);
        }
        socket.write_all(response).await.unwrap();
    });
    (reqwest::Client::new().get(url), task)
}

#[tokio::test]
async fn chunked_response_is_bounded_without_content_length() {
    let (request, server) = raw_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n12345\r\n5\r\n67890\r\n0\r\n\r\n").await;
    assert!(matches!(
        send(request, 8, false).await,
        Err(DataError::TooLarge { maximum: 8 })
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn interrupted_transfer_is_not_returned_as_successful_partial_bytes() {
    let (request, server) =
        raw_response(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n12345").await;
    assert!(matches!(
        send(request, 20, false).await,
        Err(DataError::Transport)
    ));
    server.await.unwrap();
}
