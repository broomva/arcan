use axum::{
    Router,
    extract::Request,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use std::path::Path;

// ─── Embedded assets (production) ─────────────────────────────────────────────

#[cfg(feature = "embed")]
#[derive(rust_embed::Embed)]
#[folder = "frontend/dist"]
struct ConsoleAssets;

#[cfg(feature = "embed")]
pub fn embedded_router() -> Router {
    Router::new().fallback(get(serve_embedded))
}

#[cfg(feature = "embed")]
async fn serve_embedded(req: Request) -> Response {
    let path = req.uri().path().trim_start_matches('/');

    // Try the exact path first.
    if let Some(file) = ConsoleAssets::get(path) {
        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime)],
            file.data.to_vec(),
        )
            .into_response();
    }

    // SPA fallback: return index.html for non-file routes.
    if let Some(index) = ConsoleAssets::get("index.html") {
        return Html(String::from_utf8_lossy(&index.data).into_owned()).into_response();
    }

    StatusCode::NOT_FOUND.into_response()
}

#[cfg(not(feature = "embed"))]
pub fn embedded_router() -> Router {
    Router::new().fallback(get(|| async {
        (
            StatusCode::NOT_FOUND,
            "Console assets not embedded. Build with `--features embed` or use --console-dir.",
        )
    }))
}

// ─── Filesystem assets (development) ─────────────────────────────────────────

pub fn filesystem_router(dir: &Path) -> Router {
    let serve_dir = tower_http::services::ServeDir::new(dir)
        .fallback(tower_http::services::ServeFile::new(dir.join("index.html")));
    Router::new().fallback_service(serve_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use std::io::Write as IoWrite;
    use tower::ServiceExt;

    fn temp_dir_with_index() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("create temp dir");
        let mut f = std::fs::File::create(dir.path().join("index.html")).expect("create index");
        f.write_all(b"<html><body>Arcan Console</body></html>")
            .expect("write index");

        // Write a CSS file for MIME type testing.
        let mut css = std::fs::File::create(dir.path().join("style.css")).expect("create css");
        css.write_all(b"body { color: white; }").expect("write css");

        dir
    }

    #[tokio::test]
    async fn filesystem_serves_index() {
        let dir = temp_dir_with_index();
        let router = filesystem_router(dir.path());
        let resp = router
            .oneshot(HttpRequest::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn filesystem_spa_fallback() {
        let dir = temp_dir_with_index();
        let router = filesystem_router(dir.path());
        let resp = router
            .oneshot(
                HttpRequest::builder()
                    .uri("/some/deep/route")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        assert!(
            String::from_utf8_lossy(&body).contains("Arcan Console"),
            "SPA fallback should return index.html"
        );
    }

    #[tokio::test]
    async fn filesystem_serves_static_with_mime() {
        let dir = temp_dir_with_index();
        let router = filesystem_router(dir.path());
        let resp = router
            .oneshot(
                HttpRequest::builder()
                    .uri("/style.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            ct.contains("css"),
            "CSS files should have text/css content type, got: {ct}"
        );
    }
}
