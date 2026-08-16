//! Static assets: the web UI, embedded at compile time (release) or read
//! from `web/dist` live (debug builds, via rust-embed's `debug-embed`).
//! A placeholder `index.html` lives in `web/dist` in git so a fresh clone
//! compiles before the first `pnpm build` — it tells the visitor to build.
//!
//! SPA fallback: any non-`/api` path that isn't a file serves `index.html`;
//! an unknown `/api` path gets the contract's JSON 404, never the SPA.

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Json;

#[derive(rust_embed::Embed)]
#[folder = "../../web/dist"]
struct Assets;

pub async fn static_or_spa(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"code": "not_found", "message": "route not found"})),
        )
            .into_response();
    }
    let path = if path.is_empty() { "index.html" } else { path };
    match Assets::get(path) {
        Some(file) => serve(path, &file.data),
        // SPA fallback: client-side routes render index.html.
        None => match Assets::get("index.html") {
            Some(file) => serve("index.html", &file.data),
            None => (
                StatusCode::SERVICE_UNAVAILABLE,
                "web UI not built — run `pnpm build` in web/",
            )
                .into_response(),
        },
    }
}

fn serve(path: &str, data: &[u8]) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    (
        [(header::CONTENT_TYPE, mime.as_ref())],
        data.to_vec(),
    )
        .into_response()
}
