use anyhow::Result;
use chrono::Local;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::food::{Food, Macros};

pub struct Database {
    conn: Connection,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: Option<i64>,
    pub date: String,
    pub food_name: String,
    pub food_id: i64,
    pub amount: String,
    pub protein: f64,
    pub fat: f64,
    pub carbs: f64,
    pub calories: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Stats {
    pub food_count: i64,
    pub log_count: i64,
    pub first_entry: Option<String>,
    pub last_entry: Option<String>,
}

impl Database {
    /// Open an in-memory database (for testing)
    #[allow(dead_code)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.init()?;
        Ok(db)
    }

    pub fn open() -> Result<Self> {
        let db_path = Self::db_path()?;

        // Create parent directory if needed
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(&db_path)?;
        Ok(Self { conn })
    }

    fn db_path() -> Result<std::path::PathBuf> {
        let home =
            dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?;
        Ok(home.join(".chomp").join("foods.db"))
    }

    pub fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS foods (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                protein REAL NOT NULL,
                fat REAL NOT NULL,
                carbs REAL NOT NULL,
                calories REAL NOT NULL,
                serving TEXT NOT NULL DEFAULT '100g',
                default_amount TEXT,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS aliases (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                food_id INTEGER NOT NULL,
                alias TEXT NOT NULL UNIQUE,
                FOREIGN KEY (food_id) REFERENCES foods(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                date TEXT NOT NULL,
                food_id INTEGER NOT NULL,
                amount TEXT NOT NULL,
                protein REAL NOT NULL,
                fat REAL NOT NULL,
                carbs REAL NOT NULL,
                calories REAL NOT NULL,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (food_id) REFERENCES foods(id)
            );

            CREATE TABLE IF NOT EXISTS compound_foods (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS compound_food_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                compound_food_id INTEGER NOT NULL,
                food_id INTEGER NOT NULL,
                amount TEXT NOT NULL,
                FOREIGN KEY (compound_food_id) REFERENCES compound_foods(id) ON DELETE CASCADE,
                FOREIGN KEY (food_id) REFERENCES foods(id)
            );

            CREATE INDEX IF NOT EXISTS idx_log_date ON log(date);
            CREATE INDEX IF NOT EXISTS idx_foods_name ON foods(name);
            CREATE INDEX IF NOT EXISTS idx_aliases_alias ON aliases(alias);
            ",
        )?;
        Ok(())
    }

    pub fn add_food(&self, food: &Food) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO foods (name, protein, fat, carbs, calories, serving, default_amount)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                food.name,
                food.protein,
                food.fat,
                food.carbs,
                food.calories,
                food.serving,
                food.default_amount,
            ],
        )?;

        let food_id = self.conn.last_insert_rowid();

        // Add aliases
        for alias in &food.aliases {
            self.conn.execute(
                "INSERT INTO aliases (food_id, alias) VALUES (?1, ?2)",
                params![food_id, alias],
            )?;
        }

        Ok(food_id)
    }

    pub fn get_food_by_name(&self, name: &str) -> Result<Option<Food>> {
        let name_lower = name.to_lowercase();

        // Try exact match first
        let mut stmt = self.conn.prepare(
            "SELECT id, name, protein, fat, carbs, calories, serving, default_amount 
             FROM foods WHERE LOWER(name) = ?1",
        )?;

        if let Ok(food) = stmt.query_row(params![&name_lower], |row| {
            Ok(Food {
                id: Some(row.get(0)?),
                name: row.get(1)?,
                protein: row.get(2)?,
                fat: row.get(3)?,
                carbs: row.get(4)?,
                calories: row.get(5)?,
                serving: row.get(6)?,
                default_amount: row.get(7)?,
                aliases: vec![],
            })
        }) {
            return Ok(Some(food));
        }

        // Try alias match
        let mut stmt = self.conn.prepare(
            "SELECT f.id, f.name, f.protein, f.fat, f.carbs, f.calories, f.serving, f.default_amount 
             FROM foods f
             JOIN aliases a ON f.id = a.food_id
             WHERE LOWER(a.alias) = ?1"
        )?;

        if let Ok(food) = stmt.query_row(params![&name_lower], |row| {
            Ok(Food {
                id: Some(row.get(0)?),
                name: row.get(1)?,
                protein: row.get(2)?,
                fat: row.get(3)?,
                carbs: row.get(4)?,
                calories: row.get(5)?,
                serving: row.get(6)?,
                default_amount: row.get(7)?,
                aliases: vec![],
            })
        }) {
            return Ok(Some(food));
        }

        Ok(None)
    }

    pub fn search_foods(&self, query: &str) -> Result<Vec<Food>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, protein, fat, carbs, calories, serving, default_amount FROM foods",
        )?;

        let foods: Vec<Food> = stmt
            .query_map([], |row| {
                Ok(Food {
                    id: Some(row.get(0)?),
                    name: row.get(1)?,
                    protein: row.get(2)?,
                    fat: row.get(3)?,
                    carbs: row.get(4)?,
                    calories: row.get(5)?,
                    serving: row.get(6)?,
                    default_amount: row.get(7)?,
                    aliases: vec![],
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Fuzzy match
        let matcher = SkimMatcherV2::default();
        let query_lower = query.to_lowercase();

        let mut scored: Vec<_> = foods
            .into_iter()
            .filter_map(|food| {
                let score = matcher.fuzzy_match(&food.name.to_lowercase(), &query_lower);
                score.map(|s| (s, food))
            })
            .collect();

        scored.sort_by(|a, b| b.0.cmp(&a.0));

        Ok(scored.into_iter().map(|(_, f)| f).take(10).collect())
    }

    pub fn log_food(&self, food_id: i64, amount: &str, macros: &Macros) -> Result<LogEntry> {
        let date = Local::now().format("%Y-%m-%d").to_string();

        self.conn.execute(
            "INSERT INTO log (date, food_id, amount, protein, fat, carbs, calories)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                date,
                food_id,
                amount,
                macros.protein,
                macros.fat,
                macros.carbs,
                macros.calories,
            ],
        )?;

        let id = self.conn.last_insert_rowid();

        // Get food name
        let food_name: String = self.conn.query_row(
            "SELECT name FROM foods WHERE id = ?1",
            params![food_id],
            |row| row.get(0),
        )?;

        Ok(LogEntry {
            id: Some(id),
            date,
            food_name,
            food_id,
            amount: amount.to_string(),
            protein: macros.protein,
            fat: macros.fat,
            carbs: macros.carbs,
            calories: macros.calories,
        })
    }

    pub fn get_today_totals(&self) -> Result<Macros> {
        let date = Local::now().format("%Y-%m-%d").to_string();

        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(SUM(protein), 0), COALESCE(SUM(fat), 0), 
                    COALESCE(SUM(carbs), 0), COALESCE(SUM(calories), 0)
             FROM log WHERE date = ?1",
        )?;

        let macros = stmt.query_row(params![date], |row| {
            Ok(Macros {
                protein: row.get(0)?,
                fat: row.get(1)?,
                carbs: row.get(2)?,
                calories: row.get(3)?,
            })
        })?;

        Ok(macros)
    }

    pub fn get_history(&self, days: u32) -> Result<Vec<LogEntry>> {
        let start_date = Local::now()
            .checked_sub_signed(chrono::Duration::days(days as i64))
            .unwrap()
            .format("%Y-%m-%d")
            .to_string();

        let mut stmt = self.conn.prepare(
            "SELECT l.id, l.date, f.name, l.food_id, l.amount, l.protein, l.fat, l.carbs, l.calories
             FROM log l
             JOIN foods f ON l.food_id = f.id
             WHERE l.date >= ?1
             ORDER BY l.date DESC, l.id DESC"
        )?;

        let entries = stmt
            .query_map(params![start_date], |row| {
                Ok(LogEntry {
                    id: Some(row.get(0)?),
                    date: row.get(1)?,
                    food_name: row.get(2)?,
                    food_id: row.get(3)?,
                    amount: row.get(4)?,
                    protein: row.get(5)?,
                    fat: row.get(6)?,
                    carbs: row.get(7)?,
                    calories: row.get(8)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(entries)
    }

    pub fn edit_food(
        &self,
        name: &str,
        protein: Option<f64>,
        fat: Option<f64>,
        carbs: Option<f64>,
        serving: Option<&str>,
        calories: Option<f64>,
    ) -> Result<()> {
        // Get the current food
        let food = self
            .get_food_by_name(name)?
            .ok_or_else(|| anyhow::anyhow!("Food not found: '{}'", name))?;

        // Build update query based on which fields are provided
        let mut updates = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(p) = protein {
            updates.push("protein = ?");
            params_vec.push(Box::new(p));
        }
        if let Some(f) = fat {
            updates.push("fat = ?");
            params_vec.push(Box::new(f));
        }
        if let Some(c) = carbs {
            updates.push("carbs = ?");
            params_vec.push(Box::new(c));
        }
        if let Some(s) = serving {
            updates.push("serving = ?");
            params_vec.push(Box::new(s.to_string()));
        }

        // Calculate new calories if macros changed or calories provided
        let new_protein = protein.unwrap_or(food.protein);
        let new_fat = fat.unwrap_or(food.fat);
        let new_carbs = carbs.unwrap_or(food.carbs);
        let new_calories = if let Some(c) = calories {
            c
        } else {
            (new_protein * 4.0) + (new_fat * 9.0) + (new_carbs * 4.0)
        };

        updates.push("calories = ?");
        params_vec.push(Box::new(new_calories));

        if updates.is_empty() {
            return Ok(());
        }

        // Add the name parameter for WHERE clause
        params_vec.push(Box::new(name.to_string()));

        let query = format!(
            "UPDATE foods SET {} WHERE LOWER(name) = LOWER(?)",
            updates.join(", ")
        );

        let params_refs: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

        self.conn.execute(&query, params_refs.as_slice())?;
        Ok(())
    }

    pub fn search_food(&self, name: &str) -> Result<Option<Food>> {
        self.get_food_by_name(name)
    }

    pub fn delete_food(&self, name: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM foods WHERE LOWER(name) = LOWER(?1)",
            params![name],
        )?;
        Ok(())
    }

    pub fn get_stats(&self) -> Result<Stats> {
        let food_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM foods", [], |row| row.get(0))?;

        let log_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM log", [], |row| row.get(0))?;

        let first_entry: Option<String> = self
            .conn
            .query_row("SELECT MIN(date) FROM log", [], |row| row.get(0))
            .ok();

        let last_entry: Option<String> = self
            .conn
            .query_row("SELECT MAX(date) FROM log", [], |row| row.get(0))
            .ok();

        Ok(Stats {
            food_count,
            log_count,
            first_entry,
            last_entry,
        })
    }

    pub fn export_csv(&self) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT l.date, f.name, l.amount, l.protein, l.fat, l.carbs, l.calories
             FROM log l
             JOIN foods f ON l.food_id = f.id
             ORDER BY l.date, l.id",
        )?;

        println!("date,food,amount,protein,fat,carbs,calories");

        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let date: String = row.get(0)?;
            let name: String = row.get(1)?;
            let amount: String = row.get(2)?;
            let protein: f64 = row.get(3)?;
            let fat: f64 = row.get(4)?;
            let carbs: f64 = row.get(5)?;
            let calories: f64 = row.get(6)?;

            println!(
                "{},{},{},{:.1},{:.1},{:.1},{:.0}",
                date, name, amount, protein, fat, carbs, calories
            );
        }

        Ok(())
    }

    pub fn export_json(&self) -> Result<()> {
        let entries = self.get_history(365)?;
        println!("{}", serde_json::to_string_pretty(&entries)?);
        Ok(())
    }

    pub fn import_usda(&self) -> Result<()> {
        use std::io::Read;

        println!("Downloading USDA SR Legacy dataset...");
        let url =
            "https://fdc.nal.usda.gov/fdc-datasets/FoodData_Central_sr_legacy_food_csv_2018-04.zip";
        let response = reqwest::blocking::get(url)
            .map_err(|e| anyhow::anyhow!("Failed to download USDA data: {}", e))?;

        let bytes = response
            .bytes()
            .map_err(|e| anyhow::anyhow!("Failed to read response: {}", e))?;

        println!("Extracting data...");
        let reader = std::io::Cursor::new(&bytes);
        let mut archive = zip::ZipArchive::new(reader)?;

        // Read food.csv to get food names and fdc_ids
        let mut food_csv = String::new();
        archive.by_name("food.csv")?.read_to_string(&mut food_csv)?;

        // Read food_nutrient.csv for nutrient values
        let mut nutrient_csv = String::new();
        archive
            .by_name("food_nutrient.csv")?
            .read_to_string(&mut nutrient_csv)?;

        // Parse foods: fdc_id -> description
        let mut foods: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut food_reader = csv::Reader::from_reader(food_csv.as_bytes());
        for record in food_reader.records() {
            let record = record?;
            let fdc_id = record.get(0).unwrap_or("").to_string();
            let description = record.get(2).unwrap_or("").to_string();
            if !description.is_empty() {
                foods.insert(fdc_id, description);
            }
        }

        // Nutrient IDs: 1003=protein, 1004=fat, 1005=carbs, 1008=calories
        // Parse nutrients: fdc_id -> (protein, fat, carbs, calories)
        let mut nutrients: std::collections::HashMap<String, (f64, f64, f64, f64)> =
            std::collections::HashMap::new();
        let mut nut_reader = csv::Reader::from_reader(nutrient_csv.as_bytes());
        for record in nut_reader.records() {
            let record = record?;
            let fdc_id = record.get(1).unwrap_or("").to_string();
            let nutrient_id = record.get(2).unwrap_or("");
            let amount: f64 = record.get(3).unwrap_or("0").parse().unwrap_or(0.0);

            let entry = nutrients.entry(fdc_id).or_insert((0.0, 0.0, 0.0, 0.0));
            match nutrient_id {
                "1003" => entry.0 = amount,
                "1004" => entry.1 = amount,
                "1005" => entry.2 = amount,
                "1008" => entry.3 = amount,
                _ => {}
            }
        }

        // Filter to foods that have all macros and reasonable names
        println!("Importing foods...");
        let mut count = 0;

        self.conn.execute("BEGIN", [])?;

        for (fdc_id, name) in &foods {
            if let Some(&(protein, fat, carbs, calories)) = nutrients.get(fdc_id) {
                // Skip foods with no nutritional data
                if protein == 0.0 && fat == 0.0 && carbs == 0.0 && calories == 0.0 {
                    continue;
                }
                // Skip very long or weird names
                if name.len() > 100 || name.contains("USDA") {
                    continue;
                }

                let clean_name = name.to_lowercase();
                // Title case
                let title_name: String = clean_name
                    .split_whitespace()
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            None => String::new(),
                            Some(f) => f.to_uppercase().to_string() + c.as_str(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");

                let result = self.conn.execute(
                    "INSERT OR IGNORE INTO foods (name, protein, fat, carbs, calories, serving)
                     VALUES (?1, ?2, ?3, ?4, ?5, '100g')",
                    params![title_name, protein, fat, carbs, calories],
                );

                if let Ok(changes) = result {
                    if changes > 0 {
                        count += 1;
                    }
                }
            }
        }

        self.conn.execute("COMMIT", [])?;

        println!("Imported {} foods from USDA SR Legacy", count);
        Ok(())
    }

    pub fn import_csv(&self, path: &str) -> Result<()> {
        let mut reader = csv::Reader::from_path(path)
            .map_err(|e| anyhow::anyhow!("Failed to open CSV file: {}", e))?;

        let mut count = 0;
        let mut skipped = 0;

        for record in reader.records() {
            let record = record?;

            let name = record.get(0).unwrap_or("").trim().to_string();
            let protein: f64 = record.get(1).unwrap_or("0").parse().unwrap_or(0.0);
            let fat: f64 = record.get(2).unwrap_or("0").parse().unwrap_or(0.0);
            let carbs: f64 = record.get(3).unwrap_or("0").parse().unwrap_or(0.0);
            let calories: f64 = record.get(4).unwrap_or("0").parse().unwrap_or(0.0);
            let serving = record.get(5).unwrap_or("100g").trim().to_string();

            if name.is_empty() {
                continue;
            }

            let calories = if calories == 0.0 {
                protein * 4.0 + fat * 9.0 + carbs * 4.0
            } else {
                calories
            };

            let result = self.conn.execute(
                "INSERT OR IGNORE INTO foods (name, protein, fat, carbs, calories, serving)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![name, protein, fat, carbs, calories, serving],
            );

            match result {
                Ok(changes) if changes > 0 => count += 1,
                Ok(_) => skipped += 1,
                Err(_) => skipped += 1,
            }
        }

        println!("Imported {} foods ({} skipped/duplicates)", count, skipped);
        Ok(())
    }

    pub fn delete_log_entry(&self, id: i64) -> Result<LogEntry> {
        // Get the entry before deleting for confirmation
        let entry: LogEntry = self.conn.query_row(
            "SELECT l.id, l.date, f.name, l.food_id, l.amount, l.protein, l.fat, l.carbs, l.calories
             FROM log l
             JOIN foods f ON l.food_id = f.id
             WHERE l.id = ?1",
            params![id],
            |row| {
                Ok(LogEntry {
                    id: Some(row.get(0)?),
                    date: row.get(1)?,
                    food_name: row.get(2)?,
                    food_id: row.get(3)?,
                    amount: row.get(4)?,
                    protein: row.get(5)?,
                    fat: row.get(6)?,
                    carbs: row.get(7)?,
                    calories: row.get(8)?,
                })
            },
        )?;

        self.conn
            .execute("DELETE FROM log WHERE id = ?1", params![id])?;
        Ok(entry)
    }

    pub fn delete_last_log_entry(&self) -> Result<LogEntry> {
        // Get the most recent entry
        let id: i64 =
            self.conn
                .query_row("SELECT id FROM log ORDER BY id DESC LIMIT 1", [], |row| {
                    row.get(0)
                })?;

        self.delete_log_entry(id)
    }

    pub fn edit_log_entry(
        &self,
        id: i64,
        amount: Option<String>,
        protein: Option<f64>,
        fat: Option<f64>,
        carbs: Option<f64>,
    ) -> Result<LogEntry> {
        // Get the current entry
        let entry: LogEntry = self.conn.query_row(
            "SELECT l.id, l.date, f.name, l.food_id, l.amount, l.protein, l.fat, l.carbs, l.calories
             FROM log l
             JOIN foods f ON l.food_id = f.id
             WHERE l.id = ?1",
            params![id],
            |row| {
                Ok(LogEntry {
                    id: Some(row.get(0)?),
                    date: row.get(1)?,
                    food_name: row.get(2)?,
                    food_id: row.get(3)?,
                    amount: row.get(4)?,
                    protein: row.get(5)?,
                    fat: row.get(6)?,
                    carbs: row.get(7)?,
                    calories: row.get(8)?,
                })
            },
        )?;

        // Build update query based on which fields are provided
        let mut updates = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        let new_amount = amount.clone().unwrap_or(entry.amount.clone());
        let new_protein = protein.unwrap_or(entry.protein);
        let new_fat = fat.unwrap_or(entry.fat);
        let new_carbs = carbs.unwrap_or(entry.carbs);
        let new_calories = (new_protein * 4.0) + (new_fat * 9.0) + (new_carbs * 4.0);

        if amount.is_some() {
            updates.push("amount = ?");
            params_vec.push(Box::new(new_amount.clone()));
        }
        if protein.is_some() {
            updates.push("protein = ?");
            params_vec.push(Box::new(new_protein));
        }
        if fat.is_some() {
            updates.push("fat = ?");
            params_vec.push(Box::new(new_fat));
        }
        if carbs.is_some() {
            updates.push("carbs = ?");
            params_vec.push(Box::new(new_carbs));
        }

        // Always update calories if any macro changed
        if protein.is_some() || fat.is_some() || carbs.is_some() {
            updates.push("calories = ?");
            params_vec.push(Box::new(new_calories));
        }

        if updates.is_empty() {
            return Ok(entry);
        }

        // Add the id parameter for WHERE clause
        params_vec.push(Box::new(id));

        let query = format!("UPDATE log SET {} WHERE id = ?", updates.join(", "));

        let params_refs: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

        self.conn.execute(&query, params_refs.as_slice())?;

        // Return updated entry
        Ok(LogEntry {
            id: Some(id),
            date: entry.date,
            food_name: entry.food_name,
            food_id: entry.food_id,
            amount: new_amount,
            protein: new_protein,
            fat: new_fat,
            carbs: new_carbs,
            calories: new_calories,
        })
    }

    /// Create a compound food from component foods with amounts
    /// items: Vec<(food_name, amount_str)>
    pub fn create_compound_food(&self, name: &str, items: &[(String, String)]) -> Result<()> {
        // Validate all component foods exist
        let mut resolved: Vec<(i64, String)> = Vec::new();
        for (food_name, amount) in items {
            let food = self
                .get_food_by_name(food_name)?
                .ok_or_else(|| anyhow::anyhow!("Food not found: '{}'", food_name))?;
            resolved.push((food.id.unwrap(), amount.clone()));
        }

        self.conn.execute(
            "INSERT INTO compound_foods (name) VALUES (?1)",
            params![name],
        )?;
        let compound_id = self.conn.last_insert_rowid();

        for (food_id, amount) in &resolved {
            self.conn.execute(
                "INSERT INTO compound_food_items (compound_food_id, food_id, amount) VALUES (?1, ?2, ?3)",
                params![compound_id, food_id, amount],
            )?;
        }

        // Also create a regular food entry with the summed macros
        let mut total = crate::food::Macros::default();
        for (food_name, amount) in items {
            let food = self.get_food_by_name(food_name)?.unwrap();
            if let Some(macros) = food.calculate(amount) {
                total.add(&macros);
            } else {
                // If can't calculate, use base macros
                total.add(&crate::food::Macros {
                    protein: food.protein,
                    fat: food.fat,
                    carbs: food.carbs,
                    calories: food.calories,
                });
            }
        }

        self.conn.execute(
            "INSERT OR REPLACE INTO foods (name, protein, fat, carbs, calories, serving)
             VALUES (?1, ?2, ?3, ?4, ?5, '1serving')",
            params![name, total.protein, total.fat, total.carbs, total.calories],
        )?;

        println!(
            "Created compound food '{}': {:.0}p/{:.0}f/{:.0}c — {:.0} kcal",
            name, total.protein, total.fat, total.carbs, total.calories
        );

        Ok(())
    }

    /// List compound food details
    #[allow(dead_code)]
    pub fn get_compound_food(&self, name: &str) -> Result<Vec<(String, String)>> {
        let compound_id: i64 = self.conn.query_row(
            "SELECT id FROM compound_foods WHERE LOWER(name) = LOWER(?1)",
            params![name],
            |row| row.get(0),
        )?;

        let mut stmt = self.conn.prepare(
            "SELECT f.name, ci.amount FROM compound_food_items ci
             JOIN foods f ON ci.food_id = f.id
             WHERE ci.compound_food_id = ?1",
        )?;

        let items = stmt
            .query_map(params![compound_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::food::{Food, Macros};

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    fn sample_food(name: &str) -> Food {
        Food::new(name, 26.0, 15.0, 0.0, 250.0, "100g", vec![])
    }

    #[test]
    fn test_add_and_retrieve_food() {
        let db = test_db();
        let food = sample_food("Ribeye");
        let id = db.add_food(&food).unwrap();
        assert!(id > 0);

        let found = db.get_food_by_name("ribeye").unwrap().unwrap();
        assert_eq!(found.name, "Ribeye");
        assert_eq!(found.protein, 26.0);
    }

    #[test]
    fn test_add_food_with_aliases() {
        let db = test_db();
        let food = Food::new(
            "Chicken Breast",
            31.0,
            3.6,
            0.0,
            165.0,
            "100g",
            vec!["chicken".to_string(), "chx".to_string()],
        );
        db.add_food(&food).unwrap();

        let found = db.get_food_by_name("chicken").unwrap().unwrap();
        assert_eq!(found.name, "Chicken Breast");

        let found2 = db.get_food_by_name("chx").unwrap().unwrap();
        assert_eq!(found2.name, "Chicken Breast");
    }

    #[test]
    fn test_search_foods_fuzzy() {
        let db = test_db();
        db.add_food(&sample_food("Ribeye Steak")).unwrap();
        db.add_food(&sample_food("Rice")).unwrap();
        db.add_food(&sample_food("Salmon")).unwrap();

        let results = db.search_foods("rib").unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "Ribeye Steak");
    }

    #[test]
    fn test_log_food_and_today_totals() {
        let db = test_db();
        let food = sample_food("Eggs");
        let id = db.add_food(&food).unwrap();

        let macros = Macros {
            protein: 12.0,
            fat: 10.0,
            carbs: 1.0,
            calories: 142.0,
        };
        let entry = db.log_food(id, "2", &macros).unwrap();
        assert_eq!(entry.food_name, "Eggs");
        assert_eq!(entry.protein, 12.0);

        let totals = db.get_today_totals().unwrap();
        assert_eq!(totals.protein, 12.0);
        assert_eq!(totals.calories, 142.0);

        // Log another
        let macros2 = Macros {
            protein: 26.0,
            fat: 15.0,
            carbs: 0.0,
            calories: 250.0,
        };
        db.log_food(id, "100g", &macros2).unwrap();

        let totals = db.get_today_totals().unwrap();
        assert_eq!(totals.protein, 38.0);
    }

    #[test]
    fn test_get_history() {
        let db = test_db();
        let id = db.add_food(&sample_food("Bacon")).unwrap();
        let macros = Macros {
            protein: 12.0,
            fat: 40.0,
            carbs: 0.0,
            calories: 400.0,
        };
        db.log_food(id, "100g", &macros).unwrap();

        let history = db.get_history(7).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].food_name, "Bacon");
    }

    #[test]
    fn test_edit_food() {
        let db = test_db();
        db.add_food(&sample_food("Salmon")).unwrap();

        db.edit_food("Salmon", Some(25.0), None, None, None, None)
            .unwrap();
        let food = db.get_food_by_name("Salmon").unwrap().unwrap();
        assert_eq!(food.protein, 25.0);
        // calories recalculated: 25*4 + 15*9 + 0*4 = 235
        assert_eq!(food.calories, 235.0);
    }

    #[test]
    fn test_delete_food() {
        let db = test_db();
        db.add_food(&sample_food("Temp Food")).unwrap();
        assert!(db.get_food_by_name("Temp Food").unwrap().is_some());

        db.delete_food("Temp Food").unwrap();
        assert!(db.get_food_by_name("Temp Food").unwrap().is_none());
    }

    #[test]
    fn test_delete_log_entry() {
        let db = test_db();
        let id = db.add_food(&sample_food("Apple")).unwrap();
        let macros = Macros {
            protein: 0.3,
            fat: 0.2,
            carbs: 14.0,
            calories: 52.0,
        };
        let entry = db.log_food(id, "1", &macros).unwrap();

        let deleted = db.delete_log_entry(entry.id.unwrap()).unwrap();
        assert_eq!(deleted.food_name, "Apple");

        let totals = db.get_today_totals().unwrap();
        assert_eq!(totals.calories, 0.0);
    }

    #[test]
    fn test_delete_last_log_entry() {
        let db = test_db();
        let id = db.add_food(&sample_food("Banana")).unwrap();
        let m = Macros {
            protein: 1.0,
            fat: 0.3,
            carbs: 23.0,
            calories: 89.0,
        };
        db.log_food(id, "1", &m).unwrap();
        db.log_food(id, "1", &m).unwrap();

        let deleted = db.delete_last_log_entry().unwrap();
        assert_eq!(deleted.food_name, "Banana");

        let totals = db.get_today_totals().unwrap();
        assert_eq!(totals.calories, 89.0);
    }

    #[test]
    fn test_edit_log_entry() {
        let db = test_db();
        let id = db.add_food(&sample_food("Steak")).unwrap();
        let m = Macros {
            protein: 26.0,
            fat: 15.0,
            carbs: 0.0,
            calories: 250.0,
        };
        let entry = db.log_food(id, "100g", &m).unwrap();

        let updated = db
            .edit_log_entry(
                entry.id.unwrap(),
                Some("200g".to_string()),
                Some(52.0),
                None,
                None,
            )
            .unwrap();
        assert_eq!(updated.amount, "200g");
        assert_eq!(updated.protein, 52.0);
    }

    #[test]
    fn test_get_stats() {
        let db = test_db();
        let stats = db.get_stats().unwrap();
        assert_eq!(stats.food_count, 0);
        assert_eq!(stats.log_count, 0);

        let id = db.add_food(&sample_food("Rice")).unwrap();
        let m = Macros {
            protein: 2.7,
            fat: 0.3,
            carbs: 28.0,
            calories: 130.0,
        };
        db.log_food(id, "100g", &m).unwrap();

        let stats = db.get_stats().unwrap();
        assert_eq!(stats.food_count, 1);
        assert_eq!(stats.log_count, 1);
    }

    #[test]
    fn test_duplicate_food_handling() {
        let db = test_db();
        db.add_food(&sample_food("Eggs")).unwrap();
        let result = db.add_food(&sample_food("Eggs"));
        assert!(result.is_err()); // UNIQUE constraint
    }

    #[test]
    fn test_compound_food() {
        let db = test_db();
        db.add_food(&Food::new("Rice", 2.7, 0.3, 28.0, 130.0, "100g", vec![]))
            .unwrap();
        db.add_food(&Food::new(
            "Chicken Breast",
            31.0,
            3.6,
            0.0,
            165.0,
            "100g",
            vec![],
        ))
        .unwrap();

        db.create_compound_food(
            "Chicken Rice Bowl",
            &[
                ("Rice".to_string(), "200g".to_string()),
                ("Chicken Breast".to_string(), "150g".to_string()),
            ],
        )
        .unwrap();

        let found = db.get_food_by_name("Chicken Rice Bowl").unwrap().unwrap();
        assert!(found.calories > 0.0);

        let items = db.get_compound_food("Chicken Rice Bowl").unwrap();
        assert_eq!(items.len(), 2);
    }
}
