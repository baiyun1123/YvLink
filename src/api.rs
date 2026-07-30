use std::{sync::Arc, time::Instant};

use axum::{
    Json, Router,
    extract::{Path, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{Next, from_fn_with_state},
    response::{IntoResponse, Response},
    routing::{get, put},
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    AppConfig, CrossplayConfig, CrossplayStatus, GlobalSettings, RuleConfig, RuntimeManager,
    crossplay_status,
};

#[derive(Clone)]
pub struct ApiState {
    pub manager: Arc<RuntimeManager>,
    pub admin_token: Arc<str>,
    pub started_at: Instant,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

#[derive(Serialize)]
struct ApiResponse<T> {
    ok: bool,
    data: T,
}

#[derive(Serialize)]
struct CrossplayView {
    config: CrossplayConfig,
    status: CrossplayStatus,
}

pub fn api_router(state: ApiState) -> Router {
    Router::new()
        .route("/session", get(session))
        .route("/status", get(status))
        .route("/crossplay", get(get_crossplay).put(update_crossplay))
        .route("/config", get(get_config).put(update_config))
        .route("/rules", get(list_rules).post(create_rule))
        .route("/rules/{id}", put(update_rule).delete(delete_rule))
        .layer(from_fn_with_state(state.clone(), require_auth))
        .with_state(state)
}

async fn get_crossplay(State(state): State<ApiState>) -> Json<ApiResponse<CrossplayView>> {
    let config = state.manager.config().await.crossplay;
    let status = crossplay_status(&config).await;
    success(CrossplayView { config, status })
}

async fn update_crossplay(
    State(state): State<ApiState>,
    Json(crossplay): Json<CrossplayConfig>,
) -> Result<Json<ApiResponse<CrossplayView>>, ApiError> {
    let config = state
        .manager
        .update_crossplay(crossplay)
        .await
        .map_err(ApiError::bad_request)?
        .crossplay;
    let status = crossplay_status(&config).await;
    Ok(success(CrossplayView { config, status }))
}

async fn session() -> Json<ApiResponse<Value>> {
    success(json!({ "authenticated": true }))
}

async fn status(State(state): State<ApiState>) -> Json<ApiResponse<Value>> {
    let runtime = state.manager.status().await;
    success(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": state.started_at.elapsed().as_secs(),
        "totals": runtime.totals,
        "proxy_running": runtime.proxy_running,
        "rules": runtime.rules,
    }))
}

async fn get_config(State(state): State<ApiState>) -> Json<ApiResponse<AppConfig>> {
    success(state.manager.config().await)
}

async fn update_config(
    State(state): State<ApiState>,
    Json(settings): Json<GlobalSettings>,
) -> Result<Json<ApiResponse<AppConfig>>, ApiError> {
    state
        .manager
        .update_settings(settings)
        .await
        .map(success)
        .map_err(ApiError::bad_request)
}

async fn list_rules(State(state): State<ApiState>) -> Json<ApiResponse<Vec<RuleConfig>>> {
    success(state.manager.config().await.rules)
}

async fn create_rule(
    State(state): State<ApiState>,
    Json(rule): Json<RuleConfig>,
) -> Result<(StatusCode, Json<ApiResponse<RuleConfig>>), ApiError> {
    state
        .manager
        .create_rule(rule)
        .await
        .map(|rule| (StatusCode::CREATED, success(rule)))
        .map_err(ApiError::bad_request)
}

async fn update_rule(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(rule): Json<RuleConfig>,
) -> Result<Json<ApiResponse<RuleConfig>>, ApiError> {
    state
        .manager
        .update_rule(&id, rule)
        .await
        .map(success)
        .map_err(ApiError::bad_request)
}

async fn delete_rule(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    state
        .manager
        .delete_rule(&id)
        .await
        .map(|()| success(json!({ "deleted": id })))
        .map_err(ApiError::bad_request)
}

async fn require_auth(State(state): State<ApiState>, request: Request, next: Next) -> Response {
    let supplied = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    if supplied.is_some_and(|token| constant_time_eq(token, &state.admin_token)) {
        return next.run(request).await;
    }

    let mut response = ApiError {
        status: StatusCode::UNAUTHORIZED,
        message: "管理令牌无效或缺失".to_string(),
    }
    .into_response();
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Bearer realm=\"mc-proxy-admin\""),
    );
    response
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut difference = left.len() ^ right.len();
    let length = left.len().max(right.len());
    for index in 0..length {
        let left_byte = left.get(index).copied().unwrap_or_default();
        let right_byte = right.get(index).copied().unwrap_or_default();
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}

fn success<T>(data: T) -> Json<ApiResponse<T>> {
    Json(ApiResponse { ok: true, data })
}

impl ApiError {
    fn bad_request(error: anyhow::Error) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "ok": false,
                "error": {
                    "code": self.status.as_u16(),
                    "message": self.message,
                }
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_comparison_requires_exact_match() {
        assert!(constant_time_eq("0123456789", "0123456789"));
        assert!(!constant_time_eq("0123456789", "0123456780"));
        assert!(!constant_time_eq("short", "longer"));
    }
}
