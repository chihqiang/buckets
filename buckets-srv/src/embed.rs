use axum::http::{StatusCode, Uri, header};
use axum::response::IntoResponse;
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../web/dist"]
struct Assets;

pub async fn fallback(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');

    if let Some(file) = Assets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        let body: Vec<u8> = file.data.into_owned();
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime.as_ref())],
            body,
        )
            .into_response();
    }

    if (path.is_empty() || !path.contains('.')) && let Some(index) = Assets::get("index.html") {
        let body: Vec<u8> = index.data.into_owned();
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            body,
        )
            .into_response();
    }

    (StatusCode::NOT_FOUND, "Not Found").into_response()
}
