# chomp

Local food database CLI for AI-assisted nutrition tracking.

## Problem

AI assistants waste credits searching for nutrition data every time you log food. Your diet is repetitive — the same foods show up constantly. Why look up "ribeye" for the 50th time?

## Solution

Local SQLite database that learns YOUR foods. AI queries it instead of web searching.

## Commands

```bash
# Log food (default action)
chomp bacon                      # logs bacon
chomp ribeye 8oz                 # logs 8oz ribeye
chomp "bare bar"                 # logs bare bar

# Manage foods (the database of what things are)
chomp add ribeye --protein 23 --fat 18 --carbs 0 --per 100g
chomp edit ribeye --protein 25 --fat 20
chomp delete "food name"         # removes food definition from DB

# Manage log entries (what you actually ate)
chomp unlog 42                   # delete log entry by ID
chomp unlog-last                 # delete most recent log entry
chomp edit-log 42 --amount 8oz   # fix a log entry

# Query
chomp search salmon              # fuzzy match
chomp today                      # show today's totals
chomp history                    # recent logs
chomp stats                      # database stats

# Import/Export
chomp export --csv               # for spreadsheets
chomp import usda                # seed from USDA database
```

## Implemented Features

- **Fuzzy matching** — "rib eye" = "ribeye"
- **Learned portions** — "salmon" defaults to your usual 4oz (via `default_amount` field)
- **Aliases** — "bb" = "bare bar"
- **JSON output** — All commands support `--json` for AI integration
- **MCP server** — `chomp serve` for Claude Desktop integration

- **Compound foods** — `chomp compound "breakfast" -i "3 eggs + 2 bacon"` saves multi-item meals as single entry
- **USDA import** — `chomp import usda` downloads and imports from FoodData Central SR Legacy dataset
- **CSV import** — `chomp import csv --path foods.csv` for bulk loading (header: name,protein,fat,carbs,calories,serving)

## Roadmap / Planned Features

- **Nutrition label import** — Dedicated workflow for photo → AI extraction → DB (currently works via manual `chomp add`)
- **Smart defaults** — Learn your typical portions and auto-suggest them

## AI Integration

### CLI (for OpenClaw/exec)
```bash
chomp "salmon 4oz" --json        # log + structured output
chomp search salmon --json       # nutrition lookup without web search
```

### MCP Server
```bash
# stdio transport (Claude Desktop)
chomp serve                          # default: stdio
chomp serve --transport stdio

# SSE transport (Poke.com, remote agents)
chomp serve --transport sse          # default: http://127.0.0.1:3000
chomp serve --transport sse --port 3456 --host 0.0.0.0

# Both transports simultaneously
chomp serve --transport both --port 3000
```

**Transport options:**

| Transport | Use case | Endpoint |
|-----------|----------|----------|
| `stdio` | Claude Desktop, local AI | stdin/stdout |
| `sse` | Poke.com, Railway, remote | `GET /sse` + `POST /message` |
| `both` | Run both simultaneously | stdio + HTTP |

**SSE endpoints:**
- `GET /sse` — SSE event stream (returns `endpoint` event with session POST URL)
- `POST /message?sessionId=<id>` — send JSON-RPC requests
- `GET /health` — health check

**Environment variables (SSE mode):**
```bash
CHOMP_PORT=3000          # default: 3000
CHOMP_HOST=0.0.0.0       # default: 127.0.0.1
```

Exposes tools:
- `log_food(food)` → logs + returns entry with calculated macros
- `search_food(query)` → fuzzy search results with nutrition info
- `add_food(name, protein, fat, carbs, serving, ...)` → add new food to DB
- `get_today()` → today's macro totals
- `get_history(days)` → recent log entries

## Workflows

### Daily Logging
Human tells AI what they ate → AI calls `chomp "food"` → done

### New Food from Label
Human sends photo of nutrition label → AI extracts data via vision → AI calls `chomp add` → food in DB forever

### Macro Check-ins
AI calls `chomp today --json` → reports totals without searching

## Tech Stack

- **Language:** Rust (fast, single binary, no runtime)
- **Database:** SQLite (portable, no server)
- **Optional:** Seed from USDA FoodData Central on first run

## File Locations

- DB: `~/.chomp/foods.db`

## Prior Art

- MyFitnessPal — bloated, cloud-only, privacy concerns
- Cronometer — good but no API, no CLI
- noms (Python) — nutrition data but not tracking-focused

## License

MIT
