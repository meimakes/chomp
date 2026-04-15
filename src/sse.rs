use anyhow::Result;
use axum::{
    extract::{Path, Query, Request, State},
    http::{header, Method, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive},
        Html, IntoResponse, Redirect, Response, Sse,
    },
    routing::{delete, get, post, put},
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
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers(Any);

    // All protected routes go here (before route_layer)
    let app = Router::new()
        .route("/sse", get(sse_handler))
        .route("/message", post(message_handler))
        .route("/dashboard", get(dashboard_handler))
        .route("/api/export", get(export_handler))
        .route("/api/today", get(today_handler))
        .route("/api/log", post(log_handler))
        .route(
            "/api/foods",
            get(search_foods_handler).post(add_food_handler),
        )
        .route(
            "/api/foods/:name",
            put(edit_food_handler).delete(delete_food_handler),
        )
        .route("/api/history", get(history_handler))
        .route("/api/log/last", delete(delete_last_log_handler))
        .route(
            "/api/log/:id",
            delete(delete_log_handler).put(edit_log_handler),
        )
        .route("/api/stats", get(stats_handler))
        .route(
            "/api/water",
            get(water_today_handler).post(log_water_handler),
        )
        .route("/api/water/history", get(water_history_handler))
        .route("/api/water/:id", delete(delete_water_handler))
        .route("/api/water/last", delete(delete_last_water_handler))
        .route(
            "/api/caffeine",
            get(caffeine_today_handler).post(log_caffeine_handler),
        )
        .route("/api/caffeine/history", get(caffeine_history_handler))
        .route("/api/caffeine/:id", delete(delete_caffeine_handler))
        .route("/api/caffeine/last", delete(delete_last_caffeine_handler))
        .route("/api/backup", get(backup_handler))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        // Public routes (after route_layer)
        .route("/health", get(health_handler))
        .route("/login", get(login_page_handler).post(login_handler))
        .route("/logout", post(logout_handler))
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
    eprintln!("  Dashboard:     http://{}/dashboard", addr);
    eprintln!("  Health check:  http://{}/health", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Extract the chomp_session cookie value from a request.
fn get_session_cookie(request: &Request) -> Option<String> {
    request
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                let c = c.trim();
                c.strip_prefix("chomp_session=").map(String::from)
            })
        })
}

/// Returns true if the request looks like it came from a browser expecting HTML.
fn is_browser_request(request: &Request) -> bool {
    request
        .headers()
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/html"))
        .unwrap_or(false)
}

/// Middleware that checks for a valid Bearer token or session cookie when auth is enabled.
async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    if let Some(expected_key) = &state.auth_key {
        // Check Bearer token first
        let bearer_ok = request
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(|token| token == expected_key.as_str())
            .unwrap_or(false);

        // Then check session cookie
        let cookie_ok = get_session_cookie(&request)
            .map(|token| token == *expected_key)
            .unwrap_or(false);

        if !bearer_ok && !cookie_ok {
            // Redirect browsers to login page; return 401 for API clients
            if is_browser_request(&request) {
                let path = request
                    .uri()
                    .path_and_query()
                    .map(|pq| pq.as_str())
                    .unwrap_or("/dashboard");
                let login_url = format!("/login?next={}", urlencoding::encode(path));
                return Redirect::to(&login_url).into_response();
            }
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body("Missing or invalid Authorization: Bearer <key> header".into())
                .unwrap();
        }
    }

    next.run(request).await
}

/// Helper to open DB, returning an error response on failure.
fn open_db() -> std::result::Result<Database, (StatusCode, Json<serde_json::Value>)> {
    Database::open()
        .and_then(|db| {
            db.init()?;
            Ok(db)
        })
        .map_err(|e| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": format!("database error: {}", e)})),
            )
        })
}

/// GET /sse — client connects here, receives an SSE stream.
async fn sse_handler(
    State(state): State<Arc<AppState>>,
) -> Sse<ReceiverStream<std::result::Result<Event, Infallible>>> {
    let session_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::channel(32);

    let endpoint_url = format!("/message?sessionId={}", session_id);
    let _ = tx
        .send(Ok(Event::default().event("endpoint").data(endpoint_url)))
        .await;

    let tx_clone = tx.clone();
    state.sessions.lock().await.insert(session_id.clone(), tx);

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
    let mut sessions = state.sessions.lock().await;
    let tx = match sessions.get(&query.session_id) {
        Some(tx) if tx.is_closed() => {
            sessions.remove(&query.session_id);
            return StatusCode::NOT_FOUND;
        }
        Some(tx) => tx.clone(),
        None => return StatusCode::NOT_FOUND,
    };
    drop(sessions);

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

/// GET /api/export?days=N — returns CSV of log entries.
async fn export_handler(Query(params): Query<HashMap<String, String>>) -> impl IntoResponse {
    let db = match open_db() {
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

    let days: u32 = params
        .get("days")
        .and_then(|d| d.parse().ok())
        .unwrap_or(90);

    let entries = match db.get_history(days) {
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
        // Quote food_name and amount since they may contain commas
        let food_quoted = if e.food_name.contains(',') {
            format!("\"{}\"", e.food_name.replace('"', "\"\""))
        } else {
            e.food_name.clone()
        };
        let amount_quoted = if e.amount.contains(',') {
            format!("\"{}\"", e.amount.replace('"', "\"\""))
        } else {
            e.amount.clone()
        };
        csv.push_str(&format!(
            "{},{},{},{:.1},{:.1},{:.1},{:.0}\n",
            e.date, food_quoted, amount_quoted, e.protein, e.fat, e.carbs, e.calories
        ));
    }

    (StatusCode::OK, [(header::CONTENT_TYPE, "text/csv")], csv).into_response()
}

/// GET /api/today — returns today's totals + entries as JSON.
async fn today_handler() -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    let totals = db.get_today_totals().unwrap_or_default();
    let entries = db.get_today_entries().unwrap_or_default();
    let water = db.get_today_water().unwrap_or_default();
    let caffeine = db.get_today_caffeine().unwrap_or_default();

    Json(serde_json::json!({
        "totals": totals,
        "entries": entries,
        "water": water,
        "caffeine": caffeine
    }))
    .into_response()
}

// --- REST API handlers ---

#[derive(Deserialize)]
struct LogRequest {
    food: String,
    date: Option<String>,
}

/// POST /api/log — parse and log food.
async fn log_handler(Json(body): Json<LogRequest>) -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    match crate::logging::parse_and_log(&db, &body.food, body.date.as_deref()) {
        Ok(entry) => (StatusCode::CREATED, Json(serde_json::json!(entry))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct SearchQuery {
    #[serde(rename = "q")]
    query: String,
}

/// GET /api/foods?q=query — search foods.
async fn search_foods_handler(Query(params): Query<SearchQuery>) -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    match db.search_foods(&params.query) {
        Ok(foods) => Json(serde_json::json!(foods)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct AddFoodRequest {
    name: String,
    protein: f64,
    fat: f64,
    carbs: f64,
    #[serde(default = "default_serving")]
    per: String,
    calories: Option<f64>,
    #[serde(default)]
    aliases: Vec<String>,
}

fn default_serving() -> String {
    "100g".to_string()
}

/// POST /api/foods — add a new food.
async fn add_food_handler(Json(body): Json<AddFoodRequest>) -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    let cals = body
        .calories
        .unwrap_or(body.protein * 4.0 + body.fat * 9.0 + body.carbs * 4.0);
    let food = crate::food::Food::new(
        &body.name,
        body.protein,
        body.fat,
        body.carbs,
        cals,
        &body.per,
        body.aliases,
    );

    match db.add_food(&food) {
        Ok(_) => (StatusCode::CREATED, Json(serde_json::json!(food))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct EditFoodRequest {
    protein: Option<f64>,
    fat: Option<f64>,
    carbs: Option<f64>,
    per: Option<String>,
    calories: Option<f64>,
}

/// PUT /api/foods/:name — edit a food.
async fn edit_food_handler(
    Path(name): Path<String>,
    Json(body): Json<EditFoodRequest>,
) -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    match db.edit_food(
        &name,
        body.protein,
        body.fat,
        body.carbs,
        body.per.as_deref(),
        body.calories,
    ) {
        Ok(()) => {
            let food = db.search_food(&name).ok().flatten();
            Json(serde_json::json!(food)).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// DELETE /api/foods/:name — delete a food.
async fn delete_food_handler(Path(name): Path<String>) -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    match db.delete_food(&name) {
        Ok(()) => Json(serde_json::json!({"deleted": name})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct HistoryQuery {
    days: Option<u32>,
}

/// GET /api/history?days=N — get history.
async fn history_handler(Query(params): Query<HistoryQuery>) -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    let days = params.days.unwrap_or(7);
    match db.get_history(days) {
        Ok(entries) => Json(serde_json::json!(entries)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// DELETE /api/log/:id — delete a log entry by ID.
async fn delete_log_handler(Path(id): Path<i64>) -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    match db.delete_log_entry(id) {
        Ok(entry) => Json(serde_json::json!(entry)).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// DELETE /api/log/last — delete the most recent log entry.
async fn delete_last_log_handler() -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    match db.delete_last_log_entry() {
        Ok(entry) => Json(serde_json::json!(entry)).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct EditLogRequest {
    amount: Option<String>,
    protein: Option<f64>,
    fat: Option<f64>,
    carbs: Option<f64>,
}

/// PUT /api/log/:id — edit a log entry.
async fn edit_log_handler(
    Path(id): Path<i64>,
    Json(body): Json<EditLogRequest>,
) -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    match db.edit_log_entry(id, body.amount, body.protein, body.fat, body.carbs) {
        Ok(entry) => Json(serde_json::json!(entry)).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// --- Water API handlers ---

#[derive(Deserialize)]
struct LogWaterRequest {
    amount: String,
    date: Option<String>,
}

/// POST /api/water — log water intake.
async fn log_water_handler(Json(body): Json<LogWaterRequest>) -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    let ml = match crate::food::parse_water_ml(&body.amount) {
        Some(ml) => ml,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Could not parse water amount: '{}'", body.amount)})),
            )
                .into_response()
        }
    };

    match db.log_water(ml, body.date.as_deref()) {
        Ok(entry) => (StatusCode::CREATED, Json(serde_json::json!(entry))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/water — get today's water totals.
async fn water_today_handler() -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    match db.get_today_water() {
        Ok(totals) => Json(serde_json::json!(totals)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/water/history?days=N — get water history.
async fn water_history_handler(Query(params): Query<HistoryQuery>) -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    let days = params.days.unwrap_or(7);
    match db.get_water_history(days) {
        Ok(entries) => Json(serde_json::json!(entries)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// DELETE /api/water/:id — delete a water entry.
async fn delete_water_handler(Path(id): Path<i64>) -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    match db.delete_water_entry(id) {
        Ok(entry) => Json(serde_json::json!(entry)).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// DELETE /api/water/last — delete the most recent water entry.
async fn delete_last_water_handler() -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    match db.delete_last_water_entry() {
        Ok(entry) => Json(serde_json::json!(entry)).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// --- Caffeine API handlers ---

#[derive(Deserialize)]
struct LogCaffeineRequest {
    amount_mg: f64,
    #[serde(default)]
    source: String,
    date: Option<String>,
}

/// POST /api/caffeine — log caffeine intake.
async fn log_caffeine_handler(Json(body): Json<LogCaffeineRequest>) -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    match db.log_caffeine(body.amount_mg, &body.source, body.date.as_deref()) {
        Ok(entry) => (StatusCode::CREATED, Json(serde_json::json!(entry))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/caffeine — get today's caffeine totals.
async fn caffeine_today_handler() -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    match db.get_today_caffeine() {
        Ok(totals) => Json(serde_json::json!(totals)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/caffeine/history?days=N — get caffeine history.
async fn caffeine_history_handler(Query(params): Query<HistoryQuery>) -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    let days = params.days.unwrap_or(7);
    match db.get_caffeine_history(days) {
        Ok(entries) => Json(serde_json::json!(entries)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// DELETE /api/caffeine/:id — delete a caffeine entry.
async fn delete_caffeine_handler(Path(id): Path<i64>) -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    match db.delete_caffeine_entry(id) {
        Ok(entry) => Json(serde_json::json!(entry)).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// DELETE /api/caffeine/last — delete the most recent caffeine entry.
async fn delete_last_caffeine_handler() -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    match db.delete_last_caffeine_entry() {
        Ok(entry) => Json(serde_json::json!(entry)).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/stats — get database stats.
async fn stats_handler() -> impl IntoResponse {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => return e.into_response(),
    };

    match db.get_stats() {
        Ok(stats) => Json(serde_json::json!(stats)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/backup — download the SQLite database file.
async fn backup_handler() -> impl IntoResponse {
    let db_path = match Database::db_path() {
        Ok(p) => p,
        Err(e) => return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": format!("could not determine database path: {}", e)})),
        )
            .into_response(),
    };

    match tokio::fs::read(&db_path).await {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/octet-stream"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"foods.db\"",
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("database file not found: {}", e)})),
        )
            .into_response(),
    }
}

/// GET /login — serves the login page.
async fn login_page_handler(State(state): State<Arc<AppState>>) -> Response {
    if state.auth_key.is_none() {
        // No auth configured — redirect straight to dashboard
        return Redirect::to("/dashboard").into_response();
    }
    let html = include_str!("../login.html");
    Html(html).into_response()
}

#[derive(Deserialize)]
struct LoginRequest {
    key: String,
}

/// POST /login — validates the auth key and sets a session cookie.
async fn login_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginRequest>,
) -> Response {
    let expected = match &state.auth_key {
        Some(k) => k,
        None => {
            // No auth configured — just succeed
            return StatusCode::OK.into_response();
        }
    };

    if body.key != *expected {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(
            "set-cookie",
            format!(
                "chomp_session={}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}",
                body.key,
                60 * 60 * 24 * 30 // 30 days
            ),
        )
        .body("OK".into())
        .unwrap()
}

/// POST /logout — clears the session cookie.
async fn logout_handler() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(
            "set-cookie",
            "chomp_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0",
        )
        .body("OK".into())
        .unwrap()
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
