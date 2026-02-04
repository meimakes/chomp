use anyhow::Result;
use clap::{Parser, Subcommand};

mod db;
mod food;
mod logging;
mod mcp;

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
    },
    /// Delete a food entry
    Delete {
        /// Food name to delete
        name: String,
    },
    /// Show database stats
    Stats,
    /// Start MCP server (for AI assistants like Claude Desktop)
    Serve,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    
    // Initialize database
    let db = db::Database::open()?;
    db.init()?;

    match cli.command {
        Some(Commands::Add { name, protein, fat, carbs, per, calories, alias }) => {
            let cals = calories.unwrap_or_else(|| protein * 4.0 + fat * 9.0 + carbs * 4.0);
            let food = food::Food::new(&name, protein, fat, carbs, cals, &per, alias);
            db.add_food(&food)?;
            
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&food)?);
            } else {
                println!("Added: {} ({:.0}p/{:.0}f/{:.0}c per {})", name, protein, fat, carbs, per);
            }
        }
        Some(Commands::Search { query }) => {
            let results = db.search_foods(&query)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else {
                for food in results {
                    println!("{}: {:.0}p/{:.0}f/{:.0}c per {}", 
                        food.name, food.protein, food.fat, food.carbs, food.serving);
                }
            }
        }
        Some(Commands::Today) => {
            let totals = db.get_today_totals()?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&totals)?);
            } else {
                println!("Today: {:.0}p / {:.0}f / {:.0}c — {:.0} kcal",
                    totals.protein, totals.fat, totals.carbs, totals.calories);
            }
        }
        Some(Commands::History { days }) => {
            let entries = db.get_history(days)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else {
                for entry in entries {
                    println!("{} | {} {} | {:.0}p/{:.0}f/{:.0}c",
                        entry.date, entry.amount, entry.food_name,
                        entry.protein, entry.fat, entry.carbs);
                }
            }
        }
        Some(Commands::Export { format }) => {
            match format.as_str() {
                "csv" => db.export_csv()?,
                "json" => db.export_json()?,
                _ => anyhow::bail!("Unknown format: {}", format),
            }
        }
        Some(Commands::Import { source, path }) => {
            match source.as_str() {
                "usda" => db.import_usda()?,
                "csv" => {
                    let p = path.ok_or_else(|| anyhow::anyhow!("--path required for csv import"))?;
                    db.import_csv(&p)?;
                }
                _ => anyhow::bail!("Unknown source: {}", source),
            }
        }
        Some(Commands::Edit { name }) => {
            todo!("Edit food: {}", name);
        }
        Some(Commands::Delete { name }) => {
            db.delete_food(&name)?;
            println!("Deleted: {}", name);
        }
        Some(Commands::Stats) => {
            let stats = db.get_stats()?;
            println!("Foods: {}", stats.food_count);
            println!("Log entries: {}", stats.log_count);
            println!("First entry: {}", stats.first_entry.unwrap_or_default());
            println!("Last entry: {}", stats.last_entry.unwrap_or_default());
        }
        Some(Commands::Serve) => {
            mcp::serve()?;
        }
        None => {
            // Default action: log food
            if cli.food.is_empty() {
                // No args, show today's totals
                let totals = db.get_today_totals()?;
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&totals)?);
                } else {
                    println!("Today: {:.0}p / {:.0}f / {:.0}c — {:.0} kcal",
                        totals.protein, totals.fat, totals.carbs, totals.calories);
                }
            } else {
                // Log the food
                let input = cli.food.join(" ");
                let entry = logging::parse_and_log(&db, &input)?;
                
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&entry)?);
                } else {
                    println!("Logged: {} {} — {:.0}p/{:.0}f/{:.0}c",
                        entry.amount, entry.food_name, entry.protein, entry.fat, entry.carbs);
                }
            }
        }
    }

    Ok(())
}
