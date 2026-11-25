// load PORT PATH CONNECTIONS SECONDS: keep-alive requests on each connection, one thread each.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (port, path) = (args[1].clone(), args[2].clone());
    let connections: usize = args[3].parse().unwrap();
    let seconds: u64 = args[4].parse().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let total = Arc::new(AtomicU64::new(0));
    let mut threads = Vec::new();
    for _ in 0..connections {
        let (stop, total, port, path) = (stop.clone(), total.clone(), port.clone(), path.clone());
        threads.push(std::thread::spawn(move || {
            let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
            stream.set_nodelay(true).unwrap();
            let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n");
            // The first response says how long every response is.
            stream.write_all(request.as_bytes()).unwrap();
            let mut seen = Vec::new();
            let mut buffer = [0u8; 4096];
            let length = loop {
                let n = stream.read(&mut buffer).unwrap();
                seen.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&seen).to_lowercase();
                if let Some(end) = text.find("\r\n\r\n") {
                    assert!(text.starts_with("http/1.1 200"), "{}", text);
                    let at = text.find("content-length:").unwrap() + 15;
                    let body: usize = text[at..].lines().next().unwrap().trim().parse().unwrap();
                    while seen.len() < end + 4 + body {
                        let n = stream.read(&mut buffer).unwrap();
                        seen.extend_from_slice(&buffer[..n]);
                    }
                    break end + 4 + body;
                }
            };
            let mut response = vec![0u8; length];
            let mut count = 0u64;
            while !stop.load(Ordering::Relaxed) {
                stream.write_all(request.as_bytes()).unwrap();
                stream.read_exact(&mut response).unwrap();
                count += 1;
            }
            total.fetch_add(count, Ordering::Relaxed);
        }));
    }
    let start = Instant::now();
    std::thread::sleep(Duration::from_secs(seconds));
    stop.store(true, Ordering::Relaxed);
    for thread in threads {
        thread.join().unwrap();
    }
    let rate = total.load(Ordering::Relaxed) as f64 / start.elapsed().as_secs_f64();
    println!("{rate:.0}");
}
