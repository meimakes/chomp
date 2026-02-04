# chomp

Local food database CLI for AI-assisted nutrition tracking.

## Problem

AI assistants waste credits searching for nutrition data every time you log food. Your diet is repetitive — the same foods show up constantly. Why look up "ribeye" for the 50th time?

## Solution

Local SQLite database that learns YOUR foods. AI queries it instead of web searching.

## Commands

```bash
chomp bacon                      # logs bacon (default action)
chomp ribeye 8oz                 # logs 8oz ribeye
chomp "bare bar"                 # logs bare bar

chomp add ribeye --protein 23 --fat 18 --carbs 0 --per 100g
chomp search salmon              # fuzzy match
chomp today                      # show today's totals
chomp history                    # recent logs
chomp export --csv               # for spreadsheets
chomp import usda                # seed from USDA database
```

## Smart Features

- **Fuzzy matching** — "rib eye" = "ribeye"
- **Learned portions** — "salmon" defaults to your usual 4oz
- **Aliases** — "bb" = "bare bar"
- **Compound foods** — "breakfast = 3 eggs + 2 bacon"
- **Nutrition label import** — send photo, AI extracts + adds to DB

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

## Status

✅ Core CLI working — add, log, search, today, history, export

## License

MIT
