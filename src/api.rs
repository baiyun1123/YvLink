use std::{path::PathBuf, sync::Arc, time::Instant};

use axum::{
    Json, Router,
    extract::{Path, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{Next, from_fn_with_state},
    response::{IntoResponse, Response},
    routing::{get, put},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AppConfig, CrossplayConfig, CrossplayStatus, GlobalSettings, RuleConfig, RuntimeManager,
    ViaLiteConfig, crossplay_status,
    geyser_lite::{CrossplayRuntime, GeyserLiteRuntimeStatus},
    via_lite::{ViaLiteRuntime, ViaLiteRuntimeStatus},
};

#[derive(Clone)]
pub struct ApiState {
    pub manager: Arc<RuntimeManager>,
    pub admin_token: Arc<str>,
    pub started_at: Instant,
    pub crossplay_runtime: CrossplayRuntime,
    pub via_runtime: ViaLiteRuntime,
    /// systemd 更新器写入的只读状态文件；管理 API 不执行更新命令。
    pub update_status_path: PathBuf,
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
    runtime: GeyserLiteRuntimeStatus,
}

#[derive(Serialize)]
struct ViaLiteView {
    config: ViaLiteConfig,
    runtime: ViaLiteRuntimeStatus,
}

#[derive(Debug, Deserialize, Serialize)]
struct UpdateStatusFile {
    state: String,
    message: String,
}

#[derive(Serialize)]
struct UpdateView {
    current_version: &'static str,
    status: UpdateStatusFile,
}

pub fn api_router(state: ApiState) -> Router {
    Router::new()
        .route("/session", get(session))
        .route("/status", get(status))
        .route("/crossplay", get(get_crossplay).put(update_crossplay))
        .route("/via", get(get_via).put(update_via))
        .route("/updates", get(get_updates))
        .route("/config", get(get_config).put(update_config))
        .route("/rules", get(list_rules).post(create_rule))
        .route("/rules/{id}", put(update_rule).delete(delete_rule))
        .layer(from_fn_with_state(state.clone(), require_auth))
        .with_state(state)
}

async fn get_updates(State(state): State<ApiState>) -> Json<ApiResponse<UpdateView>> {
    let status = match tokio::fs::read_to_string(&state.update_status_path).await {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|_| UpdateStatusFile {
            state: "unknown".to_string(),
            message: "自动更新状态文件格式无效；请检查更新服务日志。".to_string(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => UpdateStatusFile {
            state: "unknown".to_string(),
            message: "尚未收到自动更新器的状态；定时任务首次执行后会显示结果。".to_string(),
        },
        Err(_) => UpdateStatusFile {
            state: "unknown".to_string(),
            message: "暂时无法读取自动更新状态；请检查服务文件权限。".to_string(),
        },
    };
    success(UpdateView {
        current_version: env!("CARGO_PKG_VERSION"),
        status,
    })
}

async fn get_crossplay(State(state): State<ApiState>) -> Json<ApiResponse<CrossplayView>> {
    let config = state.manager.config().await.crossplay;
    let status = crossplay_status(&config).await;
    let runtime = state.crossplay_runtime.status().await;
    success(CrossplayView {
        config,
        status,
        runtime,
    })
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
    // 托管 GeyserLite 的启停是尽力而为：失败记录到 runtime 状态而不是回滚配置。
    let _ = state.crossplay_runtime.apply(&config).await;
    let status = crossplay_status(&config).await;
    let runtime = state.crossplay_runtime.status().await;
    Ok(success(CrossplayView {
        config,
        status,
        runtime,
    }))
}

async fn get_via(State(state): State<ApiState>) -> Json<ApiResponse<ViaLiteView>> {
    success(ViaLiteView {
        config: state.manager.config().await.via,
        runtime: state.via_runtime.status().await,
    })
}

async fn update_via(
    State(state): State<ApiState>,
    Json(via): Json<ViaLiteConfig>,
) -> Result<Json<ApiResponse<ViaLiteView>>, ApiError> {
    let config = state
        .manager
        .update_via(via)
        .await
        .map_err(ApiError::bad_request)?;
    // ViaLite 运行时不与前端代理共享地址空间；配置已原子落盘，运行时失败会在
    // runtime.error 呈现，保留控制台可用性，且拨号会保守回退到真实后端。
    let _ = state.via_runtime.apply(&config).await;
    Ok(success(ViaLiteView {
        config: config.via,
        runtime: state.via_runtime.status().await,
    }))
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
        "via": state.via_runtime.status().await,
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
    let rule = state
        .manager
        .create_rule(rule)
        .await
        .map_err(ApiError::bad_request)?;
    reconcile_via(&state).await;
    Ok((StatusCode::CREATED, success(rule)))
}

async fn update_rule(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(rule): Json<RuleConfig>,
) -> Result<Json<ApiResponse<RuleConfig>>, ApiError> {
    let rule = state
        .manager
        .update_rule(&id, rule)
        .await
        .map_err(ApiError::bad_request)?;
    reconcile_via(&state).await;
    Ok(success(rule))
}

async fn delete_rule(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    state
        .manager
        .delete_rule(&id)
        .await
        .map_err(ApiError::bad_request)?;
    reconcile_via(&state).await;
    Ok(success(json!({ "deleted": id })))
}

async fn reconcile_via(state: &ApiState) {
    let config = state.manager.config().await;
    let _ = state.via_runtime.apply(&config).await;
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
