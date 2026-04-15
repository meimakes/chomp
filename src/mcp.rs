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
                        },
                        "date": {
                            "type": "string",
                            "description": "Date to log for in YYYY-MM-DD format (defaults to today if omitted)"
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
            },
            {
                "name": "unlog",
                "description": "Delete a log entry by its ID (rowid from the log table).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Log entry ID to delete"
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "unlog_last",
                "description": "Delete the most recent log entry.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "delete_food",
                "description": "Delete a food from the database by name.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Food name to delete"
                        }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "edit_food",
                "description": "Edit a food entry. Only provided fields are updated.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Food name to edit"
                        },
                        "protein": {
                            "type": "number",
                            "description": "New protein in grams per serving"
                        },
                        "fat": {
                            "type": "number",
                            "description": "New fat in grams per serving"
                        },
                        "carbs": {
                            "type": "number",
                            "description": "New carbs in grams per serving"
                        },
                        "serving": {
                            "type": "string",
                            "description": "New serving size"
                        },
                        "calories": {
                            "type": "number",
                            "description": "New calories (recalculated from macros if not provided)"
                        }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "edit_log",
                "description": "Edit a log entry. Only provided fields are updated.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Log entry ID to edit"
                        },
                        "amount": {
                            "type": "string",
                            "description": "New amount"
                        },
                        "protein": {
                            "type": "number",
                            "description": "New protein in grams"
                        },
                        "fat": {
                            "type": "number",
                            "description": "New fat in grams"
                        },
                        "carbs": {
                            "type": "number",
                            "description": "New carbs in grams"
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "log_water",
                "description": "Log water intake. Supports ml (default), oz, cups, liters.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "amount": {
                            "type": "string",
                            "description": "Water amount, e.g. '500', '500ml', '16oz', '2 cups', '1l'"
                        },
                        "date": {
                            "type": "string",
                            "description": "Date in YYYY-MM-DD format (defaults to today)"
                        }
                    },
                    "required": ["amount"]
                }
            },
            {
                "name": "get_water_today",
                "description": "Get today's total water intake in ml.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "get_water_history",
                "description": "Get water intake history.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "days": {
                            "type": "integer",
                            "description": "Number of days to look back (default: 7)"
                        }
                    }
                }
            },
            {
                "name": "unlog_water",
                "description": "Delete a water log entry by ID.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Water log entry ID to delete"
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "log_caffeine",
                "description": "Log caffeine intake in mg with optional source.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "amount_mg": {
                            "type": "number",
                            "description": "Caffeine amount in milligrams"
                        },
                        "source": {
                            "type": "string",
                            "description": "Source of caffeine, e.g. 'coffee', 'tea', 'energy drink'"
                        },
                        "date": {
                            "type": "string",
                            "description": "Date in YYYY-MM-DD format (defaults to today)"
                        }
                    },
                    "required": ["amount_mg"]
                }
            },
            {
                "name": "get_caffeine_today",
                "description": "Get today's total caffeine intake in mg.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "get_caffeine_history",
                "description": "Get caffeine intake history.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "days": {
                            "type": "integer",
                            "description": "Number of days to look back (default: 7)"
                        }
                    }
                }
            },
            {
                "name": "unlog_caffeine",
                "description": "Delete a caffeine log entry by ID.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Caffeine log entry ID to delete"
                        }
                    },
                    "required": ["id"]
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
            let date = arguments["date"].as_str();
            let entry = parse_and_log(db, food, date)?;
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
        "unlog" => {
            let id = arguments["id"]
                .as_i64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'id' argument"))?;
            let entry = db.delete_log_entry(id)?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Deleted log entry: {} {} of {}", entry.id.unwrap_or(id), entry.amount, entry.food_name)
                }]
            }))
        }
        "unlog_last" => {
            let entry = db.delete_last_log_entry()?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Deleted last log entry: {} {} of {}", entry.id.unwrap_or(0), entry.amount, entry.food_name)
                }]
            }))
        }
        "delete_food" => {
            let name = arguments["name"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'name' argument"))?;
            db.delete_food(name)?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Deleted food: {}", name)
                }]
            }))
        }
        "edit_food" => {
            let name = arguments["name"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'name' argument"))?;
            let protein = arguments["protein"].as_f64();
            let fat = arguments["fat"].as_f64();
            let carbs = arguments["carbs"].as_f64();
            let serving = arguments["serving"].as_str();
            let calories = arguments["calories"].as_f64();
            db.edit_food(name, protein, fat, carbs, serving, calories)?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Updated food: {}", name)
                }]
            }))
        }
        "edit_log" => {
            let id = arguments["id"]
                .as_i64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'id' argument"))?;
            let amount = arguments["amount"].as_str().map(String::from);
            let protein = arguments["protein"].as_f64();
            let fat = arguments["fat"].as_f64();
            let carbs = arguments["carbs"].as_f64();
            let entry = db.edit_log_entry(id, amount, protein, fat, carbs)?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&entry)?
                }]
            }))
        }
        "log_water" => {
            let amount = arguments["amount"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'amount' argument"))?;
            let ml = crate::food::parse_water_ml(amount)
                .ok_or_else(|| anyhow::anyhow!("Could not parse water amount: '{}'", amount))?;
            let date = arguments["date"].as_str();
            let entry = db.log_water(ml, date)?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Logged {:.0}ml water ({:.1} oz)", entry.amount_ml, entry.amount_ml / 29.5735)
                }]
            }))
        }
        "get_water_today" => {
            let totals = db.get_today_water()?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&totals)?
                }]
            }))
        }
        "get_water_history" => {
            let days = arguments["days"].as_u64().unwrap_or(7) as u32;
            let entries = db.get_water_history(days)?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&entries)?
                }]
            }))
        }
        "unlog_water" => {
            let id = arguments["id"]
                .as_i64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'id' argument"))?;
            let entry = db.delete_water_entry(id)?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Deleted water entry: {:.0}ml on {}", entry.amount_ml, entry.date)
                }]
            }))
        }
        "log_caffeine" => {
            let amount_mg = arguments["amount_mg"]
                .as_f64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'amount_mg' argument"))?;
            let source = arguments["source"].as_str().unwrap_or("");
            let date = arguments["date"].as_str();
            let entry = db.log_caffeine(amount_mg, source, date)?;
            let src = if entry.source.is_empty() {
                String::new()
            } else {
                format!(" ({})", entry.source)
            };
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Logged {:.0}mg caffeine{}", entry.amount_mg, src)
                }]
            }))
        }
        "get_caffeine_today" => {
            let totals = db.get_today_caffeine()?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&totals)?
                }]
            }))
        }
        "get_caffeine_history" => {
            let days = arguments["days"].as_u64().unwrap_or(7) as u32;
            let entries = db.get_caffeine_history(days)?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&entries)?
                }]
            }))
        }
        "unlog_caffeine" => {
            let id = arguments["id"]
                .as_i64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'id' argument"))?;
            let entry = db.delete_caffeine_entry(id)?;
            let src = if entry.source.is_empty() {
                String::new()
            } else {
                format!(" ({})", entry.source)
            };
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Deleted caffeine entry: {:.0}mg{} on {}", entry.amount_mg, src, entry.date)
                }]
            }))
        }
        _ => Err(anyhow::anyhow!("Unknown tool: {}", tool_name)),
    }
}
