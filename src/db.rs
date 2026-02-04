use anyhow::Result;
use chrono::{Local, NaiveDate};
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
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?;
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

            CREATE INDEX IF NOT EXISTS idx_log_date ON log(date);
            CREATE INDEX IF NOT EXISTS idx_foods_name ON foods(name);
            CREATE INDEX IF NOT EXISTS idx_aliases_alias ON aliases(alias);
            "
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
             FROM foods WHERE LOWER(name) = ?1"
        )?;
        
        if let Some(food) = stmt.query_row(params![&name_lower], |row| {
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
        }).ok() {
            return Ok(Some(food));
        }
        
        // Try alias match
        let mut stmt = self.conn.prepare(
            "SELECT f.id, f.name, f.protein, f.fat, f.carbs, f.calories, f.serving, f.default_amount 
             FROM foods f
             JOIN aliases a ON f.id = a.food_id
             WHERE LOWER(a.alias) = ?1"
        )?;
        
        if let Some(food) = stmt.query_row(params![&name_lower], |row| {
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
        }).ok() {
            return Ok(Some(food));
        }
        
        Ok(None)
    }

    pub fn search_foods(&self, query: &str) -> Result<Vec<Food>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, protein, fat, carbs, calories, serving, default_amount FROM foods"
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
             FROM log WHERE date = ?1"
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

    pub fn delete_food(&self, name: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM foods WHERE LOWER(name) = LOWER(?1)",
            params![name],
        )?;
        Ok(())
    }

    pub fn get_stats(&self) -> Result<Stats> {
        let food_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM foods",
            [],
            |row| row.get(0),
        )?;
        
        let log_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM log",
            [],
            |row| row.get(0),
        )?;
        
        let first_entry: Option<String> = self.conn.query_row(
            "SELECT MIN(date) FROM log",
            [],
            |row| row.get(0),
        ).ok();
        
        let last_entry: Option<String> = self.conn.query_row(
            "SELECT MAX(date) FROM log",
            [],
            |row| row.get(0),
        ).ok();
        
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
             ORDER BY l.date, l.id"
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
            
            println!("{},{},{},{:.1},{:.1},{:.1},{:.0}", 
                date, name, amount, protein, fat, carbs, calories);
        }
        
        Ok(())
    }

    pub fn export_json(&self) -> Result<()> {
        let entries = self.get_history(365)?;
        println!("{}", serde_json::to_string_pretty(&entries)?);
        Ok(())
    }

    pub fn import_usda(&self) -> Result<()> {
        // TODO: Implement USDA FoodData Central import
        println!("USDA import not yet implemented");
        Ok(())
    }

    pub fn import_csv(&self, path: &str) -> Result<()> {
        // TODO: Implement CSV import
        println!("CSV import from {} not yet implemented", path);
        Ok(())
    }
}
