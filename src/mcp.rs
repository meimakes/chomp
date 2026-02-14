use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

use crate::db::Database;
use crate::food::Food;
use crate::logging::parse_and_log;

const SERVER_NAME: &str = "chomp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

/// Handle a JSON-RPC request and return a response.
/// Returns None for notifications (no id) that don't need a response.
pub fn handle_request(db: &Database, request: &JsonRpcRequest) -> Option<JsonRpcResponse> {
    // Per JSON-RPC 2.0 spec, requests without an id are notifications
    // and MUST NOT receive a response.
    let id = match &request.id {
        Some(id) => id.clone(),
        None => return None,
    };

    let result = match request.method.as_str() {
        "initialize" => handle_initialize(),
        "tools/list" => handle_tools_list(),
        "tools/call" => handle_tools_call(db, &request.params),
        _ => Err(anyhow::anyhow!("Method not found: {}", request.method)),
    };

    Some(match result {
        Ok(value) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(value),
            error: None,
        },
        Err(e) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code: -32603,
                message: e.to_string(),
            }),
        },
    })
}

/// Parse a JSON line into a request, returning an error response on failure.
pub fn parse_request(line: &str) -> std::result::Result<JsonRpcRequest, JsonRpcResponse> {
    serde_json::from_str(line).map_err(|e| JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: Value::Null,
        result: None,
        error: Some(JsonRpcError {
            code: -32700,
            message: format!("Parse error: {}", e),
        }),
    })
}

/// Run the MCP server over stdio transport.
pub fn serve_stdio() -> Result<()> {
    let db = Database::open()?;
    db.init()?;

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        match parse_request(&line) {
            Ok(request) => {
                if let Some(response) = handle_request(&db, &request) {
                    writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
                    stdout.flush()?;
                }
            }
            Err(error_response) => {
                writeln!(stdout, "{}", serde_json::to_string(&error_response)?)?;
                stdout.flush()?;
            }
        }
    }

    Ok(())
}

fn handle_initialize() -> Result<Value> {
    Ok(json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": SERVER_NAME,
            "version": SERVER_VERSION
        }
    }))
}

fn handle_tools_list() -> Result<Value> {
    Ok(json!({
        "tools": [
            {
                "name": "log_food",
                "description": "Log food consumption. Returns calculated macros.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "food": {
                            "type": "string",
                            "description": "Food name and optional amount, e.g. 'salmon 4oz' or 'bare bar'"
                        }
                    },
                    "required": ["food"]
                }
            },
            {
                "name": "search_food",
                "description": "Search for foods in the database. Returns matching foods with nutrition info.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query (fuzzy matching supported)"
                        }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "add_food",
                "description": "Add a new food to the database.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Food name"
                        },
                        "protein": {
                            "type": "number",
                            "description": "Protein in grams per serving"
                        },
                        "fat": {
                            "type": "number",
                            "description": "Fat in grams per serving"
                        },
                        "carbs": {
                            "type": "number",
                            "description": "Carbs in grams per serving"
                        },
                        "serving": {
                            "type": "string",
                            "description": "Serving size, e.g. '100g', '1 bar', '4oz'"
                        },
                        "calories": {
                            "type": "number",
                            "description": "Calories per serving (calculated if not provided)"
                        },
                        "aliases": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Alternative names for this food"
                        }
                    },
                    "required": ["name", "protein", "fat", "carbs", "serving"]
                }
            },
            {
                "name": "get_today",
                "description": "Get today's nutrition totals.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "get_history",
                "description": "Get recent food log entries.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "days": {
                            "type": "integer",
                            "description": "Number of days to look back (default: 7)"
                        }
                    }
                }
            }
        ]
    }))
}

fn handle_tools_call(db: &Database, params: &Value) -> Result<Value> {
    let tool_name = params["name"].as_str().unwrap_or("");
    let arguments = &params["arguments"];

    match tool_name {
        "log_food" => {
            let food = arguments["food"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'food' argument"))?;
            let entry = parse_and_log(db, food)?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&entry)?
                }]
            }))
        }
        "search_food" => {
            let query = arguments["query"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'query' argument"))?;
            let results = db.search_foods(query)?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&results)?
                }]
            }))
        }
        "add_food" => {
            let name = arguments["name"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'name' argument"))?;
            let protein = arguments["protein"]
                .as_f64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'protein' argument"))?;
            let fat = arguments["fat"]
                .as_f64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'fat' argument"))?;
            let carbs = arguments["carbs"]
                .as_f64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'carbs' argument"))?;
            let serving = arguments["serving"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'serving' argument"))?;
            let calories = arguments["calories"]
                .as_f64()
                .unwrap_or(protein * 4.0 + fat * 9.0 + carbs * 4.0);
            let aliases: Vec<String> = arguments["aliases"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            let food = Food::new(name, protein, fat, carbs, calories, serving, aliases);
            db.add_food(&food)?;

            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Added: {} ({:.0}p/{:.0}f/{:.0}c per {})",
                        name, protein, fat, carbs, serving)
                }]
            }))
        }
        "get_today" => {
            let totals = db.get_today_totals()?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&totals)?
                }]
            }))
        }
        "get_history" => {
            let days = arguments["days"].as_u64().unwrap_or(7) as u32;
            let entries = db.get_history(days)?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&entries)?
                }]
            }))
        }
        _ => Err(anyhow::anyhow!("Unknown tool: {}", tool_name)),
    }
}
