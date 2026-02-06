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

# Manage foods
chomp add ribeye --protein 23 --fat 18 --carbs 0 --per 100g
chomp edit ribeye --protein 25 --fat 20
chomp delete "food name"

# Query
chomp search salmon              # fuzzy match
chomp today                      # show today's totals
chomp history                    # recent logs

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

## Roadmap / Planned Features

Features mentioned but not yet implemented:

- **Compound foods** — "breakfast = 3 eggs + 2 bacon" (save multi-item meals as single entry)
- **USDA import** — `chomp import usda` to seed database from FoodData Central
- **CSV import** — `chomp import csv --path foods.csv` for bulk loading
- **Nutrition label import** — Dedicated workflow for photo → AI extraction → DB (currently works via manual `chomp add`)
- **Smart defaults** — Learn your typical portions and auto-suggest them

## AI Integration

### CLI (for OpenClaw/exec)
```bash
chomp "salmon 4oz" --json        # log + structured output
chomp search salmon --json       # nutrition lookup without web search
```

### MCP Server (for Claude Desktop)
```bash
chomp serve --mcp               # starts MCP server on stdio
```

Exposes tools:
- `lookup_food(name)` → nutrition JSON
- `log_food(food, amount)` → logs + returns entry
- `get_totals(date)` → day's macros
- `search_foods(query)` → fuzzy results
- `add_food(name, protein, fat, carbs, per)` → add new food

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
- Config: `~/.chomp/config.toml`
- Logs: `~/.chomp/logs/YYYY-MM-DD.json`

## Prior Art

- MyFitnessPal — bloated, cloud-only, privacy concerns
- Cronometer — good but no API, no CLI
- noms (Python) — nutrition data but not tracking-focused

## License

MIT
