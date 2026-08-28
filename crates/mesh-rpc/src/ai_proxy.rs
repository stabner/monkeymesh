//! Reverse-proxy AI brain routes from an edge node to the seed (`MESH_AI_UPSTREAM`).
//! Non-brain board routes (`advertise`/`job`/`result`) stay local on hybrid edges.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::any;
use axum::Router;

use crate::RpcState;

/// Full AI surface proxied (legacy / `MESH_EDGE_AI_LOCAL=0`).
pub fn ai_proxy_router() -> Router<RpcState> {
    Router::new()
        .route("/v1/advertise", any(proxy))
        .route("/v1/job", any(proxy))
        .route("/v1/result", any(proxy))
        .route("/v1/results", any(proxy))
        .route("/v1/workers", any(proxy))
        .route("/v1/research/status", any(proxy))
        .route("/v1/research/scenarios", any(proxy))
        .route("/v1/ai/health", any(proxy))
        .merge(ai_brain_proxy_router())
}

/// Seed-canonical brain / trilemma routes only (hybrid edge).
pub fn ai_brain_proxy_router() -> Router<RpcState> {
    Router::new()
        .route("/v1/model", any(proxy))
        .route("/v1/model/meta", any(proxy))
        .route("/v1/model/bin", any(proxy))
        .route("/v1/trilemma", any(proxy))
        .route("/v1/quantum", any(proxy))
        .route("/v1/leg/{*rest}", any(proxy))
        .route("/v1/qleg/{*rest}", any(proxy))
}

async fn proxy(
    State(st): State<RpcState>,
    req: Request,
) -> Result<Response, (StatusCode, String)> {
    let Some(upstream) = st.ai_upstream.as_ref() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "MESH_AI_UPSTREAM not set".into(),
        ));
    };
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(req.uri().path());
    let url = format!("{upstream}{path_and_query}");
    let method = req.method().clone();
    let headers = filter_headers(req.headers());
    let body_bytes = axum::body::to_bytes(req.into_body(), 32 * 1024 * 1024)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    let mut builder = client.request(method, &url);
    for (k, v) in headers.iter() {
        builder = builder.header(k, v);
    }
    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes.to_vec());
    }

    let upstream_resp = builder
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream: {e}")))?;

    let status = StatusCode::from_u16(upstream_resp.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let mut out_headers = HeaderMap::new();
    for (k, v) in upstream_resp.headers().iter() {
        if k == axum::http::header::TRANSFER_ENCODING || k == axum::http::header::CONNECTION {
            continue;
        }
        if let Ok(v) = HeaderValue::from_bytes(v.as_bytes()) {
            out_headers.insert(k.clone(), v);
        }
    }
    let bytes = upstream_resp
        .bytes()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    let mut resp = Response::new(Body::from(bytes));
    *resp.status_mut() = status;
    *resp.headers_mut() = out_headers;
    Ok(resp)
}

fn filter_headers(src: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (k, v) in src.iter() {
        if k == axum::http::header::HOST
            || k == axum::http::header::CONTENT_LENGTH
            || k == axum::http::header::TRANSFER_ENCODING
            || k == axum::http::header::CONNECTION
        {
            continue;
        }
        out.insert(k.clone(), v.clone());
    }
    out
}
