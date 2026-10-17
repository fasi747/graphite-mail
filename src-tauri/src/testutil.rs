//! Shared test helpers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Minimal Graph stand-in: serves canned responses by path and records every request path.
pub struct Mock {
    pub base: String,
    pub hits: Arc<Mutex<Vec<String>>>,
}

pub async fn mock_graph(routes: Vec<(&str, u16, Vec<u8>)>) -> Mock {
    let table: HashMap<String, (u16, Vec<u8>)> = routes.into_iter().map(|(p, s, b)| (p.to_string(), (s, b))).collect();
    let hits = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let h = hits.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let (table, h) = (table.clone(), h.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 2048];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let line = String::from_utf8_lossy(&buf).lines().next().unwrap_or_default().to_string();
                let method = line.split_whitespace().next().unwrap_or("").to_string();
                let path = line.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap().to_string();
                h.lock().unwrap().push(format!("{method} {path}"));
                let (status, body) = table.get(&path).cloned().unwrap_or((404, b"{}".to_vec()));
                let head = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(&body).await;
            });
        }
    });
    Mock { base, hits }
}

pub fn json_body(v: serde_json::Value) -> Vec<u8> {
    v.to_string().into_bytes()
}

