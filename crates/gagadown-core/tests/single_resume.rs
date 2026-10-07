use gagadown_core::task::Status;
use gagadown_core::{AddRequest, Engine};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const N: usize = 3 << 20;
const CUT: usize = (3 << 20) / 2 + 12345;

fn body() -> Vec<u8> {
    (0..N).map(|i| (i % 251) as u8).collect()
}

async fn serve(mut s: tokio::net::TcpStream, data: Arc<Vec<u8>>, full: Arc<AtomicUsize>) {
    let mut req = Vec::new();
    let mut b = [0u8; 1024];
    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
        match s.read(&mut b).await {
            Ok(0) | Err(_) => return,
            Ok(n) => req.extend_from_slice(&b[..n]),
        }
    }
    let req = String::from_utf8_lossy(&req).to_ascii_lowercase();
    let range = req.lines().find_map(|l| l.strip_prefix("range: bytes=")).map(|r| r.trim().to_string());
    let common = "Content-Type: application/octet-stream\r\nETag: \"v1\"\r\nConnection: close\r\n";
    match range.as_deref() {
        // Probe: claim no range support so the engine takes the single-stream path.
        Some("0-0") => {
            let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {N}\r\n{common}\r\n").as_bytes()).await;
        }
        Some(r) => {
            let from: usize = r.trim_end_matches('-').parse().unwrap();
            assert!(req.contains("if-range: \"v1\""));
            let head = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {from}-{}/{N}\r\n{common}\r\n", N - from, N - 1);
            let _ = s.write_all(head.as_bytes()).await;
            let _ = s.write_all(&data[from..]).await;
        }
        None => {
            full.fetch_add(1, Ordering::SeqCst);
            let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {N}\r\n{common}\r\n").as_bytes()).await;
            let _ = s.write_all(&data[..CUT]).await;
            let _ = s.flush().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn single_stream_resumes_after_drop() {
    let data = Arc::new(body());
    let full = Arc::new(AtomicUsize::new(0));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    {
        let (data, full) = (data.clone(), full.clone());
        tokio::spawn(async move {
            while let Ok((s, _)) = l.accept().await {
                tokio::spawn(serve(s, data.clone(), full.clone()));
            }
        });
    }
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::new(Some(tmp.path().join("data"))).unwrap();
    let out = engine
        .add(AddRequest { url: format!("http://127.0.0.1:{port}/f.bin"), dir: Some(tmp.path().join("dl")), ..Default::default() }, None)
        .unwrap();
    let view = loop {
        let v = engine.views(0).into_iter().find(|v| v.id == out.id).unwrap();
        if matches!(v.status, Status::Completed | Status::Failed) {
            break v;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert!(matches!(view.status, Status::Completed), "{:?}", view.error);
    assert_eq!(full.load(Ordering::SeqCst), 1, "should resume with Range instead of restarting");
    assert_eq!(std::fs::read(view.path.unwrap()).unwrap(), *data);
}
