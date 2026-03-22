use anyhow::Result;
use axum::{
    extract::{Query, Request, State},
    http::{header, Method, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive},
        Html, IntoResponse, Response, Sse,
    },
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::{Any, CorsLayer};

use crate::db::Database;
use crate::mcp::{self, JsonRpcRequest};

/// Per-session sender for SSE events.
type SessionTx = mpsc::Sender<std::result::Result<Event, Infallible>>;

/// Shared state across all handlers.
struct AppState {
    sessions: Mutex<HashMap<String, SessionTx>>,
    auth_key: Option<String>,
}

#[derive(Deserialize)]
struct MessageQuery {
    #[serde(rename = "sessionId")]
    session_id: String,
}

/// Start the SSE MCP server on the given port/host.
pub async fn serve_sse(port: u16, host: &str, auth_key: Option<&str>) -> Result<()> {
    let state = Arc::new(AppState {
        sessions: Mutex::new(HashMap::new()),
        auth_key: auth_key.map(String::from),
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);

    let app = Router::new()
        .route("/sse", get(sse_handler))
        .route("/message", post(message_handler))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .route("/health", get(health_handler))
        .route("/dashboard", get(dashboard_handler))
        .route("/api/export", get(export_handler))
        .route("/api/today", get(today_handler))
        .layer(cors)
        .with_state(state);

    let addr: std::net::SocketAddr = format!("{}:{}", host, port).parse()?;
    eprintln!("chomp MCP server (SSE) listening on http://{}", addr);
    if auth_key.is_some() {
        eprintln!("  Auth:          enabled (Bearer token required)");
    } else {
        eprintln!("  Auth:          disabled (use --auth-key to enable)");
    }
    eprintln!("  SSE endpoint:  http://{}/sse", addr);
    eprintln!("  POST endpoint: http://{}/message", addr);
    eprintln!("  Health check:  http://{}/health", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Middleware that checks for a valid Bearer token when auth is enabled.
async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    if let Some(expected_key) = &state.auth_key {
        let auth_header = request
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok());

        match auth_header {
            Some(header) if header.starts_with("Bearer ") => {
                let token = &header[7..];
                if token != expected_key.as_str() {
                    return Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .body("Invalid auth key".into())
                        .unwrap();
                }
            }
            _ => {
                return Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body("Missing Authorization: Bearer <key> header".into())
                    .unwrap();
            }
        }
    }

    next.run(request).await
}

/// GET /sse — client connects here, receives an SSE stream.
/// First event is `endpoint` with the POST URL containing the session ID.
async fn sse_handler(
    State(state): State<Arc<AppState>>,
) -> Sse<ReceiverStream<std::result::Result<Event, Infallible>>> {
    let session_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::channel(32);

    // Send the endpoint event so the client knows where to POST
    let endpoint_url = format!("/message?sessionId={}", session_id);
    let _ = tx
        .send(Ok(Event::default().event("endpoint").data(endpoint_url)))
        .await;

    // Store session
    let tx_clone = tx.clone();
    state.sessions.lock().await.insert(session_id.clone(), tx);

    // Clean up on disconnect: periodically check if the sender's receiver is gone
    let state_clone = state.clone();
    let sid = session_id.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            if tx_clone.is_closed() {
                state_clone.sessions.lock().await.remove(&sid);
                break;
            }
        }
    });

    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}

/// POST /message?sessionId=xxx — client sends JSON-RPC requests here.
async fn message_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MessageQuery>,
    Json(request): Json<JsonRpcRequest>,
) -> StatusCode {
    // Lazy cleanup: check if session is dead before processing
    let mut sessions = state.sessions.lock().await;
    let tx = match sessions.get(&query.session_id) {
        Some(tx) if tx.is_closed() => {
            sessions.remove(&query.session_id);
            return StatusCode::NOT_FOUND;
        }
        Some(tx) => tx.clone(),
        None => return StatusCode::NOT_FOUND,
    };
    drop(sessions); // Release lock before blocking DB work

    // Open a fresh DB connection per request (SQLite handles concurrent readers)
    let db = match Database::open().and_then(|db| {
        db.init()?;
        Ok(db)
    }) {
        Ok(db) => db,
        Err(err) => {
            eprintln!("Database error in message_handler: {}", err);
            return StatusCode::SERVICE_UNAVAILABLE;
        }
    };

    if let Some(response) = mcp::handle_request(&db, &request) {
        let json = match serde_json::to_string(&response) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("Failed to serialize JSON-RPC response: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR;
            }
        };

        let event = Event::default().event("message").data(json);
        if tx.send(Ok(event)).await.is_err() {
            eprintln!("SSE client disconnected, could not deliver response");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    }

    StatusCode::ACCEPTED
}

/// GET /dashboard — serves the chomp dashboard HTML.
async fn dashboard_handler() -> impl IntoResponse {
    let html = include_str!("../dashboard.html");
    Html(html)
}

/// GET /api/export — returns CSV of all log entries for the dashboard.
async fn export_handler() -> impl IntoResponse {
    let db = match Database::open().and_then(|db| {
        db.init()?;
        Ok(db)
    }) {
        Ok(db) => db,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::CONTENT_TYPE, "text/plain")],
                "Database error".to_string(),
            )
                .into_response();
        }
    };

    let entries = match db.get_history(90) {
        Ok(e) => e,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "text/plain")],
                "Export error".to_string(),
            )
                .into_response();
        }
    };

    let mut csv = String::from("date,food,amount,protein,fat,carbs,calories\n");
    for e in &entries {
        csv.push_str(&format!(
            "{},{},{},{:.1},{:.1},{:.1},{:.0}\n",
            e.date, e.food_name, e.amount, e.protein, e.fat, e.carbs, e.calories
        ));
    }

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/csv")],
        csv,
    )
        .into_response()
}

/// GET /api/today — returns today's totals + entries as JSON.
async fn today_handler() -> impl IntoResponse {
    let db = match Database::open().and_then(|db| {
        db.init()?;
        Ok(db)
    }) {
        Ok(db) => db,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "database error"})),
            )
                .into_response();
        }
    };

    let totals = db.get_today_totals().unwrap_or_default();
    let entries = db.get_history(1).unwrap_or_default();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "totals": totals,
            "entries": entries
        })),
    )
        .into_response()
}

/// GET /health — simple health check.
async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "transport": "sse",
        "server": "chomp",
        "version": env!("CARGO_PKG_VERSION")
    }))
}
