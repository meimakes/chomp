# chomp

Local food database CLI for AI-assisted nutrition tracking.

Built by [@meimakes](https://x.com/meimakes)


## Problem

AI assistants waste credits searching for nutrition data every time you log food. Your diet is repetitive — the same foods show up constantly. Why look up "ribeye" for the 50th time?

## Solution

Local SQLite database that learns YOUR foods. AI queries it instead of web searching. Includes a web dashboard, REST API, and MCP server for Claude Desktop integration.

## Commands

```bash
# Log food (default action)
chomp bacon                      # logs bacon (1 serving)
chomp ribeye 8oz                 # logs 8oz ribeye
chomp "bare bar"                 # logs bare bar
chomp "Ortiz Sardines" 0.5       # logs half a serving (bare number = serving multiplier)
chomp --date 2026-03-21 ribeye 8oz  # backdate to a specific day

# Manage foods (the database of what things are)
chomp add ribeye --protein 23 --fat 18 --carbs 0 --per 100g
chomp add ribeye -p 23 -f 18 -c 0 --per 100g --alias rib
chomp edit ribeye --protein 25 --fat 20
chomp delete "food name"         # removes food definition from DB

# Manage log entries (what you actually ate)
chomp unlog 42                   # delete log entry by ID
chomp unlog-last                 # delete most recent log entry
chomp edit-log 42 --amount 8oz   # fix a log entry

# Query
chomp search salmon              # fuzzy match
chomp today                      # show today's totals
chomp history                    # recent logs (default 7 days)
chomp history --days 30          # recent logs (30 days)
chomp stats                      # database stats

# Compound foods
chomp compound "breakfast" -i "3 eggs + 2 bacon"

# Import/Export
chomp export --csv               # for spreadsheets
chomp export --json              # structured output
chomp import usda                # seed from USDA database
chomp import csv --path foods.csv

# Server
chomp serve                          # MCP server (stdio)
chomp serve --transport sse          # HTTP server with SSE, REST API, dashboard
chomp serve --transport sse --auth-key mysecret  # with authentication
chomp serve --transport both         # stdio + HTTP simultaneously
```

All commands support `--json` for structured output.

## Features

- **Fuzzy matching** — "rib eye" = "ribeye"
- **Learned portions** — "salmon" defaults to your usual 4oz (via `default_amount` field)
- **Flexible amounts** — bare numbers are serving multipliers (`0.5` of a `4oz` serving = 2oz), units work too (`8oz`, `3 tbsp`)
- **Aliases** — "bb" = "bare bar"
- **Compound foods** — save multi-item meals as single entries
- **Web dashboard** — dark-themed nutrition dashboard with charts, target tracking, and daily breakdowns
- **REST API** — full CRUD API for foods and log entries
- **MCP server** — stdio and SSE transports for Claude Desktop / remote AI agents
- **Authentication** — bearer token + session cookie auth for the HTTP server
- **Remote client** — point the CLI at a remote chomp server instead of a local DB
- **USDA import** — seed from FoodData Central SR Legacy dataset (~7,800 foods)
- **CSV import** — bulk load from CSV (header: `name,protein,fat,carbs,calories,serving`)
- **Docker/Railway ready** — multi-stage Dockerfile with persistent volume support

## Web Dashboard

When running with `--transport sse`, a dashboard is available at `/dashboard`.

Features: daily calorie/protein averages, macro ratio donut chart, target tracking bars, day-of-week breakdowns, today's entries table, top foods by frequency.

Dashboard targets are configurable via URL params:

```
/dashboard?protein=120&calories=1500&calorieMode=under
```

| Param | Default | Description |
|-------|---------|-------------|
| `protein` | 100 | Daily protein target (g) |
| `calories` | 2000 | Daily calorie target |
| `calorieMode` | `under` | `under` or `over` — whether hitting target means staying below or above |

## AI Integration

### CLI (for exec-based agents)
```bash
chomp "salmon 4oz" --json        # log + structured output
chomp search salmon --json       # nutrition lookup without web search
```

### MCP Server
```bash
# stdio transport (Claude Desktop)
chomp serve
chomp serve --transport stdio

# SSE transport (remote agents, Railway)
chomp serve --transport sse
chomp serve --transport sse --port 3456 --host 0.0.0.0

# Both transports simultaneously
chomp serve --transport both --port 3000
```

**MCP tools exposed:**

| Tool | Description |
|------|-------------|
| `log_food(food, date?)` | Log food, returns entry with calculated macros |
| `search_food(query)` | Fuzzy search with nutrition info |
| `add_food(name, protein, fat, carbs, serving, ...)` | Add new food to DB |
| `edit_food(name, ...)` | Edit an existing food |
| `delete_food(name)` | Delete a food from DB |
| `get_today()` | Today's macro totals |
| `get_history(days?)` | Recent log entries |
| `unlog(id)` | Delete a log entry by ID |
| `unlog_last()` | Delete most recent log entry |
| `edit_log(id, ...)` | Edit a log entry |

### REST API

All endpoints (except `/health`, `/login`, `/logout`) require authentication via `Authorization: Bearer <key>` header or session cookie.

```
GET    /dashboard          # web dashboard
GET    /health             # health check
GET    /login              # login page
POST   /login              # authenticate (sets session cookie)
POST   /logout             # clear session

GET    /api/today           # today's totals + entries
GET    /api/history?days=7  # log history
GET    /api/export?days=30  # CSV export
POST   /api/log             # log food  { "food": "ribeye 8oz", "date": "2026-03-21" }
DELETE /api/log/:id         # delete log entry
DELETE /api/log/last        # delete most recent log entry
PUT    /api/log/:id         # edit log entry

GET    /api/foods?q=salmon  # search foods
POST   /api/foods           # add food
PUT    /api/foods/:name     # edit food
DELETE /api/foods/:name     # delete food

GET    /api/stats           # database stats
```

### Remote Client Mode

Point the CLI at a remote chomp server instead of using a local database:

```bash
export CHOMP_SERVER_URL=https://your-chomp.railway.app
export CHOMP_AUTH_KEY=your-secret-key
chomp today                      # queries the remote server
```

## Deployment (Docker / Railway)

```bash
docker build -t chomp .
docker run -p 3000:3000 -e CHOMP_AUTH_KEY=secret -v chomp-data:/data chomp
```

On Railway:
1. Deploy from GitHub
2. Set `CHOMP_AUTH_KEY` env var
3. Add a volume mounted at `/data` (via Command Palette or `railway volume add --mount-path /data`)

The Dockerfile reads Railway's `PORT` env var automatically.

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CHOMP_DB_PATH` | `~/.chomp/foods.db` | Database file path |
| `CHOMP_PORT` | `3000` | HTTP server port |
| `CHOMP_HOST` | `127.0.0.1` | HTTP server bind address |
| `CHOMP_AUTH_KEY` | _(none)_ | Authentication key for HTTP server |
| `CHOMP_SERVER_URL` | _(none)_ | Remote server URL (enables client mode) |
| `PORT` | _(none)_ | Railway-injected port (maps to `CHOMP_PORT`) |

## Tech Stack

- **Language:** Rust (fast, single binary, no runtime)
- **Database:** SQLite via rusqlite (portable, no server)
- **HTTP:** Axum + Tower (SSE, REST API, static files)
- **Charts:** Chart.js (dashboard)

## File Locations

- DB: `~/.chomp/foods.db` (local), `/data/foods.db` (Docker/Railway)

## Prior Art

- MyFitnessPal — bloated, cloud-only, privacy concerns
- Cronometer — good but no API, no CLI
- noms (Python) — nutrition data but not tracking-focused

## License

MIT
