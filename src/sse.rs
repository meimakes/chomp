use anyhow::Result;
use axum::{
    extract::{Query, State},
    http::{Method, StatusCode},
    response::{
        sse::{Event, KeepAlive},
        Sse,
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
}

#[derive(Deserialize)]
struct MessageQuery {
    #[serde(rename = "sessionId")]
    session_id: String,
}

/// Start the SSE MCP server on the given port/host.
pub async fn serve_sse(port: u16, host: &str) -> Result<()> {
    let state = Arc::new(AppState {
        sessions: Mutex::new(HashMap::new()),
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);

    let app = Router::new()
        .route("/sse", get(sse_handler))
        .route("/message", post(message_handler))
        .route("/health", get(health_handler))
        .layer(cors)
        .with_state(state);

    let addr: std::net::SocketAddr = format!("{}:{}", host, port).parse()?;
    eprintln!("chomp MCP server (SSE) listening on http://{}", addr);
    eprintln!("  SSE endpoint:  http://{}/sse", addr);
    eprintln!("  POST endpoint: http://{}/message", addr);
    eprintln!("  Health check:  http://{}/health", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
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
        .send(Ok(Event::default()
            .event("endpoint")
            .data(endpoint_url)))
        .await;

    // Store session
    state.sessions.lock().await.insert(session_id.clone(), tx);

    // Clean up on disconnect (when rx is dropped, the stream ends)
    let state_clone = state.clone();
    let sid = session_id.clone();
    tokio::spawn(async move {
        // Wait until the receiver is dropped (client disconnected)
        tokio::time::sleep(tokio::time::Duration::from_secs(86400)).await;
        state_clone.sessions.lock().await.remove(&sid);
    });

    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}

/// POST /message?sessionId=xxx — client sends JSON-RPC requests here.
async fn message_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MessageQuery>,
    Json(request): Json<JsonRpcRequest>,
) -> StatusCode {
    let sessions = state.sessions.lock().await;
    let tx = match sessions.get(&query.session_id) {
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
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };

    if let Some(response) = mcp::handle_request(&db, &request) {
        let json = match serde_json::to_string(&response) {
            Ok(j) => j,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
        };

        let event = Event::default().event("message").data(json);
        let _ = tx.send(Ok(event)).await;
    }

    StatusCode::ACCEPTED
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
