use axum::{
    Router,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};

use crate::api::{ApiState, api_router};

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const STYLES_CSS: &str = include_str!("../web/styles.css");
const API_DOCS: &str = include_str!("../docs/api.html");

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/assets/app.js", get(javascript))
        .route("/assets/styles.css", get(stylesheet))
        .route("/docs/api", get(api_docs))
        .route("/healthz", get(health))
        .nest("/api/v1", api_router(state))
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn api_docs() -> Html<&'static str> {
    Html(API_DOCS)
}

async fn javascript() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        APP_JS,
    )
        .into_response()
}

async fn stylesheet() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLES_CSS,
    )
        .into_response()
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}
