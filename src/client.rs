use anyhow::{anyhow, Result};
use reqwest::blocking::Client;
use reqwest::StatusCode;

use crate::db::{LogEntry, Stats};

fn encode_path(s: &str) -> String {
    s.replace('%', "%25")
        .replace(' ', "%20")
        .replace('/', "%2F")
        .replace('?', "%3F")
        .replace('#', "%23")
        .replace('&', "%26")
}
use crate::food::{Food, Macros};

pub struct RemoteClient {
    base_url: String,
    auth_key: String,
    client: Client,
}

impl RemoteClient {
    pub fn new(base_url: &str, auth_key: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            auth_key: auth_key.to_string(),
            client: Client::new(),
        }
    }

    fn get(&self, path: &str) -> reqwest::blocking::RequestBuilder {
        self.client
            .get(format!("{}{}", self.base_url, path))
            .bearer_auth(&self.auth_key)
    }

    fn post(&self, path: &str) -> reqwest::blocking::RequestBuilder {
        self.client
            .post(format!("{}{}", self.base_url, path))
            .bearer_auth(&self.auth_key)
    }

    fn put(&self, path: &str) -> reqwest::blocking::RequestBuilder {
        self.client
            .put(format!("{}{}", self.base_url, path))
            .bearer_auth(&self.auth_key)
    }

    fn delete(&self, path: &str) -> reqwest::blocking::RequestBuilder {
        self.client
            .delete(format!("{}{}", self.base_url, path))
            .bearer_auth(&self.auth_key)
    }

    fn check_response(resp: reqwest::blocking::Response) -> Result<reqwest::blocking::Response> {
        if resp.status() == StatusCode::UNAUTHORIZED {
            return Err(anyhow!("Authentication failed. Check CHOMP_AUTH_KEY."));
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(anyhow!("Server error ({}): {}", status, body));
        }
        Ok(resp)
    }

    pub fn log_food(&self, input: &str, date: Option<&str>) -> Result<LogEntry> {
        let mut body = serde_json::json!({"food": input});
        if let Some(d) = date {
            body["date"] = serde_json::Value::String(d.to_string());
        }
        let resp = self.post("/api/log").json(&body).send()?;
        let resp = Self::check_response(resp)?;
        Ok(resp.json()?)
    }

    pub fn search_foods(&self, query: &str) -> Result<Vec<Food>> {
        let resp = self.get("/api/foods").query(&[("q", query)]).send()?;
        let resp = Self::check_response(resp)?;
        Ok(resp.json()?)
    }

    pub fn add_food(
        &self,
        name: &str,
        protein: f64,
        fat: f64,
        carbs: f64,
        per: &str,
        calories: Option<f64>,
        aliases: Vec<String>,
    ) -> Result<Food> {
        let mut body = serde_json::json!({
            "name": name,
            "protein": protein,
            "fat": fat,
            "carbs": carbs,
            "per": per,
            "aliases": aliases,
        });
        if let Some(c) = calories {
            body["calories"] = serde_json::json!(c);
        }
        let resp = self.post("/api/foods").json(&body).send()?;
        let resp = Self::check_response(resp)?;
        Ok(resp.json()?)
    }

    pub fn edit_food(
        &self,
        name: &str,
        protein: Option<f64>,
        fat: Option<f64>,
        carbs: Option<f64>,
        per: Option<&str>,
        calories: Option<f64>,
    ) -> Result<Option<Food>> {
        let mut body = serde_json::Map::new();
        if let Some(p) = protein {
            body.insert("protein".into(), serde_json::json!(p));
        }
        if let Some(f) = fat {
            body.insert("fat".into(), serde_json::json!(f));
        }
        if let Some(c) = carbs {
            body.insert("carbs".into(), serde_json::json!(c));
        }
        if let Some(s) = per {
            body.insert("per".into(), serde_json::json!(s));
        }
        if let Some(c) = calories {
            body.insert("calories".into(), serde_json::json!(c));
        }
        let resp = self
            .put(&format!("/api/foods/{}", encode_path(name)))
            .json(&serde_json::Value::Object(body))
            .send()?;
        let resp = Self::check_response(resp)?;
        Ok(resp.json()?)
    }

    pub fn delete_food(&self, name: &str) -> Result<()> {
        let resp = self
            .delete(&format!("/api/foods/{}", encode_path(name)))
            .send()?;
        Self::check_response(resp)?;
        Ok(())
    }

    pub fn get_today_totals(&self) -> Result<Macros> {
        let resp = self.get("/api/today").send()?;
        let resp = Self::check_response(resp)?;
        let data: serde_json::Value = resp.json()?;
        Ok(serde_json::from_value(data["totals"].clone())?)
    }

    pub fn get_history(&self, days: u32) -> Result<Vec<LogEntry>> {
        let resp = self
            .get("/api/history")
            .query(&[("days", days.to_string())])
            .send()?;
        let resp = Self::check_response(resp)?;
        Ok(resp.json()?)
    }

    pub fn delete_log_entry(&self, id: i64) -> Result<LogEntry> {
        let resp = self.delete(&format!("/api/log/{}", id)).send()?;
        let resp = Self::check_response(resp)?;
        Ok(resp.json()?)
    }

    pub fn delete_last_log_entry(&self) -> Result<LogEntry> {
        let resp = self.delete("/api/log/last").send()?;
        let resp = Self::check_response(resp)?;
        Ok(resp.json()?)
    }

    pub fn edit_log_entry(
        &self,
        id: i64,
        amount: Option<String>,
        protein: Option<f64>,
        fat: Option<f64>,
        carbs: Option<f64>,
    ) -> Result<LogEntry> {
        let mut body = serde_json::Map::new();
        if let Some(a) = amount {
            body.insert("amount".into(), serde_json::json!(a));
        }
        if let Some(p) = protein {
            body.insert("protein".into(), serde_json::json!(p));
        }
        if let Some(f) = fat {
            body.insert("fat".into(), serde_json::json!(f));
        }
        if let Some(c) = carbs {
            body.insert("carbs".into(), serde_json::json!(c));
        }
        let resp = self
            .put(&format!("/api/log/{}", id))
            .json(&serde_json::Value::Object(body))
            .send()?;
        let resp = Self::check_response(resp)?;
        Ok(resp.json()?)
    }

    pub fn get_stats(&self) -> Result<Stats> {
        let resp = self.get("/api/stats").send()?;
        let resp = Self::check_response(resp)?;
        Ok(resp.json()?)
    }
}
