use anyhow::Result;
use clap::{Parser, Subcommand};

mod client;
mod db;
mod food;
mod logging;
mod mcp;
#[cfg(feature = "sse")]
mod sse;

#[derive(Parser)]
#[command(name = "chomp")]
#[command(about = "Local food database for AI-assisted nutrition tracking")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Food to log (default action)
    #[arg(trailing_var_arg = true)]
    food: Vec<String>,

    /// Date to log for (YYYY-MM-DD format, defaults to today)
    #[arg(long)]
    date: Option<String>,

    /// Output as JSON
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new food to the database
    Add {
        /// Food name
        name: String,
        /// Protein in grams
        #[arg(long, short)]
        protein: f64,
        /// Fat in grams
        #[arg(long, short)]
        fat: f64,
        /// Carbs in grams
        #[arg(long, short)]
        carbs: f64,
        /// Serving size (e.g., "100g", "1 bar", "3oz")
        #[arg(long, default_value = "100g")]
        per: String,
        /// Calories (calculated if not provided)
        #[arg(long)]
        calories: Option<f64>,
        /// Aliases for this food
        #[arg(long, short)]
        alias: Vec<String>,
    },
    /// Search foods in database
    Search {
        /// Search query
        query: String,
    },
    /// Show today's totals
    Today,
    /// Show recent log entries
    History {
        /// Number of days to show
        #[arg(short, long, default_value = "7")]
        days: u32,
    },
    /// Export data
    Export {
        /// Export format
        #[arg(long, default_value = "csv")]
        format: String,
    },
    /// Import from USDA or other sources
    Import {
        /// Source (usda, csv)
        source: String,
        /// Path for csv import
        #[arg(long)]
        path: Option<String>,
    },
    /// Edit a food entry
    Edit {
        /// Food name to edit
        name: String,
        /// Protein in grams
        #[arg(long, short)]
        protein: Option<f64>,
        /// Fat in grams
        #[arg(long, short)]
        fat: Option<f64>,
        /// Carbs in grams
        #[arg(long, short)]
        carbs: Option<f64>,
        /// Serving size (e.g., "100g", "1 bar", "3oz")
        #[arg(long)]
        per: Option<String>,
        /// Calories (calculated if not provided)
        #[arg(long)]
        calories: Option<f64>,
    },
    /// Delete a food entry
    Delete {
        /// Food name to delete
        name: String,
    },
    /// Delete a log entry by ID
    Unlog {
        /// Log entry ID to delete
        id: i64,
    },
    /// Delete the most recent log entry
    UnlogLast,
    /// Edit a log entry
    EditLog {
        /// Log entry ID to edit
        id: i64,
        /// New amount
        #[arg(long)]
        amount: Option<String>,
        /// New protein in grams
        #[arg(long, short)]
        protein: Option<f64>,
        /// New fat in grams
        #[arg(long, short)]
        fat: Option<f64>,
        /// New carbs in grams
        #[arg(long, short)]
        carbs: Option<f64>,
    },
    /// Create a compound food (e.g., "breakfast = 3 eggs + 2 bacon")
    Compound {
        /// Name for the compound food
        name: String,
        /// Components in format "amount food + amount food" (e.g., "3 eggs + 2 bacon")
        #[arg(long, short = 'i')]
        items: String,
    },
    /// Show database stats
    Stats,
    /// Start MCP server (for AI assistants like Claude Desktop)
    Serve {
        /// Transport mode: stdio, sse, or both
        #[arg(long, default_value = "stdio")]
        transport: String,
        /// Port for SSE server (env: CHOMP_PORT)
        #[arg(long, default_value_t = 3000, env = "CHOMP_PORT")]
        port: u16,
        /// Host for SSE server (env: CHOMP_HOST)
        #[arg(long, default_value = "127.0.0.1", env = "CHOMP_HOST")]
        host: String,
        /// Auth key required for SSE connections (env: CHOMP_AUTH_KEY)
        #[arg(long, env = "CHOMP_AUTH_KEY")]
        auth_key: Option<String>,
    },
}

/// Backend for dispatching commands — local DB or remote server.
enum Backend {
    Local(db::Database),
    Remote(client::RemoteClient),
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Commands that always use local mode
    match &cli.command {
        Some(Commands::Serve {
            transport,
            port,
            host,
            auth_key,
        }) => {
            return run_serve(transport, *port, host, auth_key.as_deref());
        }
        Some(Commands::Import { source, path }) => {
            let db = db::Database::open()?;
            db.init()?;
            return run_import(&db, source, path.as_deref());
        }
        _ => {}
    }

    // Determine backend
    let backend = if let Ok(server_url) = std::env::var("CHOMP_SERVER_URL") {
        let auth_key = std::env::var("CHOMP_AUTH_KEY").unwrap_or_default();
        Backend::Remote(client::RemoteClient::new(&server_url, &auth_key))
    } else {
        let db = db::Database::open()?;
        db.init()?;
        Backend::Local(db)
    };

    match cli.command {
        Some(Commands::Add {
            name,
            protein,
            fat,
            carbs,
            per,
            calories,
            alias,
        }) => {
            let cals = calories.unwrap_or(protein * 4.0 + fat * 9.0 + carbs * 4.0);
            match &backend {
                Backend::Local(db) => {
                    let food = food::Food::new(&name, protein, fat, carbs, cals, &per, alias);
                    db.add_food(&food)?;
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&food)?);
                    } else {
                        println!(
                            "Added: {} ({:.0}p/{:.0}f/{:.0}c per {})",
                            name, protein, fat, carbs, per
                        );
                    }
                }
                Backend::Remote(client) => {
                    let food =
                        client.add_food(&name, protein, fat, carbs, &per, calories, alias)?;
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&food)?);
                    } else {
                        println!(
                            "Added: {} ({:.0}p/{:.0}f/{:.0}c per {})",
                            food.name, food.protein, food.fat, food.carbs, food.serving
                        );
                    }
                }
            }
        }
        Some(Commands::Search { query }) => {
            let results = match &backend {
                Backend::Local(db) => db.search_foods(&query)?,
                Backend::Remote(client) => client.search_foods(&query)?,
            };
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else {
                for food in results {
                    println!(
                        "{}: {:.0}p/{:.0}f/{:.0}c per {}",
                        food.name, food.protein, food.fat, food.carbs, food.serving
                    );
                }
            }
        }
        Some(Commands::Today) => {
            let totals = match &backend {
                Backend::Local(db) => db.get_today_totals()?,
                Backend::Remote(client) => client.get_today_totals()?,
            };
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&totals)?);
            } else {
                println!(
                    "Today: {:.0}p / {:.0}f / {:.0}c — {:.0} kcal",
                    totals.protein, totals.fat, totals.carbs, totals.calories
                );
            }
        }
        Some(Commands::History { days }) => {
            let entries = match &backend {
                Backend::Local(db) => db.get_history(days)?,
                Backend::Remote(client) => client.get_history(days)?,
            };
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else {
                for entry in entries {
                    println!(
                        "{} | {} {} | {:.0}p/{:.0}f/{:.0}c",
                        entry.date,
                        entry.amount,
                        entry.food_name,
                        entry.protein,
                        entry.fat,
                        entry.carbs
                    );
                }
            }
        }
        Some(Commands::Export { format }) => match &backend {
            Backend::Local(db) => match format.as_str() {
                "csv" => db.export_csv()?,
                "json" => db.export_json()?,
                _ => anyhow::bail!("Unknown format: {}", format),
            },
            Backend::Remote(_) => {
                anyhow::bail!("Export is only available in local mode");
            }
        },
        Some(Commands::Edit {
            name,
            protein,
            fat,
            carbs,
            per,
            calories,
        }) => match &backend {
            Backend::Local(db) => {
                db.edit_food(&name, protein, fat, carbs, per.as_deref(), calories)?;
                let food = db.search_food(&name)?;
                if let Some(f) = food {
                    println!(
                        "Updated: {} ({}p/{}f/{}c per {})",
                        f.name, f.protein, f.fat, f.carbs, f.serving
                    );
                }
            }
            Backend::Remote(client) => {
                let food =
                    client.edit_food(&name, protein, fat, carbs, per.as_deref(), calories)?;
                if let Some(f) = food {
                    println!(
                        "Updated: {} ({}p/{}f/{}c per {})",
                        f.name, f.protein, f.fat, f.carbs, f.serving
                    );
                }
            }
        },
        Some(Commands::Delete { name }) => {
            match &backend {
                Backend::Local(db) => db.delete_food(&name)?,
                Backend::Remote(client) => client.delete_food(&name)?,
            }
            println!("Deleted: {}", name);
        }
        Some(Commands::Unlog { id }) => {
            let entry = match &backend {
                Backend::Local(db) => db.delete_log_entry(id)?,
                Backend::Remote(client) => client.delete_log_entry(id)?,
            };
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&entry)?);
            } else {
                println!(
                    "Deleted log entry: {} {} — {:.0}p/{:.0}f/{:.0}c",
                    entry.amount, entry.food_name, entry.protein, entry.fat, entry.carbs
                );
            }
        }
        Some(Commands::UnlogLast) => {
            let entry = match &backend {
                Backend::Local(db) => db.delete_last_log_entry()?,
                Backend::Remote(client) => client.delete_last_log_entry()?,
            };
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&entry)?);
            } else {
                println!(
                    "Deleted last log entry: {} {} — {:.0}p/{:.0}f/{:.0}c",
                    entry.amount, entry.food_name, entry.protein, entry.fat, entry.carbs
                );
            }
        }
        Some(Commands::EditLog {
            id,
            amount,
            protein,
            fat,
            carbs,
        }) => {
            let entry = match &backend {
                Backend::Local(db) => db.edit_log_entry(id, amount, protein, fat, carbs)?,
                Backend::Remote(client) => {
                    client.edit_log_entry(id, amount, protein, fat, carbs)?
                }
            };
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&entry)?);
            } else {
                println!(
                    "Updated log entry: {} {} — {:.0}p/{:.0}f/{:.0}c",
                    entry.amount, entry.food_name, entry.protein, entry.fat, entry.carbs
                );
            }
        }
        Some(Commands::Compound { name, items }) => match &backend {
            Backend::Local(db) => {
                let parts: Vec<(String, String)> = items
                    .split('+')
                    .map(|part| {
                        let part = part.trim();
                        let words: Vec<&str> = part.split_whitespace().collect();
                        if words.len() >= 2 {
                            let amount = words[0].to_string();
                            let food = words[1..].join(" ");
                            (food, format!("{}{}", amount, "serving"))
                        } else {
                            (part.to_string(), "1serving".to_string())
                        }
                    })
                    .collect();
                db.create_compound_food(&name, &parts)?;
            }
            Backend::Remote(_) => {
                anyhow::bail!("Compound food creation is only available in local mode");
            }
        },
        Some(Commands::Stats) => {
            let stats = match &backend {
                Backend::Local(db) => db.get_stats()?,
                Backend::Remote(client) => client.get_stats()?,
            };
            println!("Foods: {}", stats.food_count);
            println!("Log entries: {}", stats.log_count);
            println!("First entry: {}", stats.first_entry.unwrap_or_default());
            println!("Last entry: {}", stats.last_entry.unwrap_or_default());
        }
        // Serve and Import handled above
        Some(Commands::Serve { .. }) | Some(Commands::Import { .. }) => unreachable!(),
        None => {
            // Default action: log food
            if cli.food.is_empty() {
                let totals = match &backend {
                    Backend::Local(db) => db.get_today_totals()?,
                    Backend::Remote(client) => client.get_today_totals()?,
                };
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&totals)?);
                } else {
                    println!(
                        "Today: {:.0}p / {:.0}f / {:.0}c — {:.0} kcal",
                        totals.protein, totals.fat, totals.carbs, totals.calories
                    );
                }
            } else {
                let input = cli.food.join(" ");
                let entry = match &backend {
                    Backend::Local(db) => logging::parse_and_log(db, &input, cli.date.as_deref())?,
                    Backend::Remote(client) => client.log_food(&input, cli.date.as_deref())?,
                };
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&entry)?);
                } else {
                    println!(
                        "Logged: {} {} — {:.0}p/{:.0}f/{:.0}c",
                        entry.amount, entry.food_name, entry.protein, entry.fat, entry.carbs
                    );
                }
            }
        }
    }

    Ok(())
}

fn run_serve(transport: &str, port: u16, host: &str, auth_key: Option<&str>) -> Result<()> {
    match transport {
        "stdio" => mcp::serve_stdio()?,
        #[cfg(feature = "sse")]
        "sse" => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(sse::serve_sse(port, host, auth_key))?;
        }
        #[cfg(feature = "sse")]
        "both" => {
            let host_clone = host.to_string();
            let auth_key_clone = auth_key.map(String::from);
            let sse_handle = std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                rt.block_on(sse::serve_sse(port, &host_clone, auth_key_clone.as_deref()))
            });
            std::thread::sleep(std::time::Duration::from_millis(100));
            if sse_handle.is_finished() {
                match sse_handle.join() {
                    Ok(Err(e)) => anyhow::bail!("SSE server failed to start: {}", e),
                    Err(_) => anyhow::bail!("SSE server thread panicked"),
                    Ok(Ok(())) => anyhow::bail!("SSE server exited unexpectedly"),
                }
            }
            mcp::serve_stdio()?;
        }
        #[cfg(not(feature = "sse"))]
        "sse" | "both" => {
            anyhow::bail!(
                "SSE transport requires the 'sse' feature. Rebuild with: cargo build --features sse"
            );
        }
        _ => anyhow::bail!("Invalid transport: {}. Use stdio, sse, or both.", transport),
    }
    Ok(())
}

fn run_import(db: &db::Database, source: &str, path: Option<&str>) -> Result<()> {
    match source {
        "usda" => db.import_usda()?,
        "csv" => {
            let p = path.ok_or_else(|| anyhow::anyhow!("--path required for csv import"))?;
            db.import_csv(p)?;
        }
        _ => anyhow::bail!("Unknown source: {}", source),
    }
    Ok(())
}
