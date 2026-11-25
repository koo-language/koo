use axum::{extract::Path, routing::get, Router};

fn main() {
    let workers: usize = std::env::var("WORKERS").ok().and_then(|w| w.parse().ok()).unwrap_or(0);
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    if workers > 0 {
        builder.worker_threads(workers);
    }
    builder.enable_all().build().unwrap().block_on(async {
        let app = Router::new()
            .route("/", get(|| async { "Hello, world!" }))
            .route("/hello/:name", get(|Path(name): Path<String>| async move { format!("Hello, {name}!") }));
        let listener = tokio::net::TcpListener::bind("0.0.0.0:18092").await.unwrap();
        axum::serve(listener, app).await.unwrap();
    });
}
