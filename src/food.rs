use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Food {
    pub id: Option<i64>,
    pub name: String,
    pub protein: f64,
    pub fat: f64,
    pub carbs: f64,
    pub calories: f64,
    pub serving: String,
    pub aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_amount: Option<String>,
}

impl Food {
    pub fn new(
        name: &str,
        protein: f64,
        fat: f64,
        carbs: f64,
        calories: f64,
        serving: &str,
        aliases: Vec<String>,
    ) -> Self {
        Self {
            id: None,
            name: name.to_string(),
            protein,
            fat,
            carbs,
            calories,
            serving: serving.to_string(),
            aliases,
            default_amount: None,
        }
    }

    /// Calculate macros for a given amount
    pub fn calculate(&self, amount: &str) -> Option<Macros> {
        let multiplier = parse_amount_multiplier(amount, &self.serving)?;
        Some(Macros {
            protein: self.protein * multiplier,
            fat: self.fat * multiplier,
            carbs: self.carbs * multiplier,
            calories: self.calories * multiplier,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Macros {
    pub protein: f64,
    pub fat: f64,
    pub carbs: f64,
    pub calories: f64,
}

impl Default for Macros {
    fn default() -> Self {
        Self {
            protein: 0.0,
            fat: 0.0,
            carbs: 0.0,
            calories: 0.0,
        }
    }
}

impl Macros {
    pub fn add(&mut self, other: &Macros) {
        self.protein += other.protein;
        self.fat += other.fat;
        self.carbs += other.carbs;
        self.calories += other.calories;
    }
}

/// Parse amount string and return multiplier relative to serving size
/// e.g., "8oz" with serving "100g" -> calculate ratio
fn parse_amount_multiplier(amount: &str, serving: &str) -> Option<f64> {
    let (amount_val, amount_unit) = parse_quantity(amount)?;
    let (serving_val, serving_unit) = parse_quantity(serving)?;
    
    // Convert both to grams for comparison
    let amount_grams = to_grams(amount_val, &amount_unit)?;
    let serving_grams = to_grams(serving_val, &serving_unit)?;
    
    Some(amount_grams / serving_grams)
}

fn parse_quantity(s: &str) -> Option<(f64, String)> {
    let s = s.trim().to_lowercase();
    
    // Handle special cases like "1 bar", "1 piece"
    if let Some(num_end) = s.find(|c: char| !c.is_numeric() && c != '.') {
        let num_str = &s[..num_end];
        let unit = s[num_end..].trim().to_string();
        let num: f64 = num_str.parse().ok()?;
        Some((num, unit))
    } else {
        // Just a number, assume grams
        let num: f64 = s.parse().ok()?;
        Some((num, "g".to_string()))
    }
}

fn to_grams(value: f64, unit: &str) -> Option<f64> {
    let unit = unit.to_lowercase();
    match unit.as_str() {
        "g" | "gram" | "grams" => Some(value),
        "oz" | "ounce" | "ounces" => Some(value * 28.3495),
        "lb" | "lbs" | "pound" | "pounds" => Some(value * 453.592),
        "kg" | "kilogram" | "kilograms" => Some(value * 1000.0),
        "ml" | "milliliter" | "milliliters" => Some(value), // Assume 1:1 for liquids
        "cup" | "cups" => Some(value * 240.0), // Approximate
        "tbsp" | "tablespoon" | "tablespoons" => Some(value * 15.0),
        "tsp" | "teaspoon" | "teaspoons" => Some(value * 5.0),
        // For discrete items (bar, piece, etc.), treat as 1:1 multiplier
        "bar" | "bars" | "piece" | "pieces" | "serving" | "servings" | "scoop" | "scoops" => Some(value * 100.0),
        _ => Some(value), // Unknown unit, assume grams
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_quantity() {
        assert_eq!(parse_quantity("100g"), Some((100.0, "g".to_string())));
        assert_eq!(parse_quantity("8oz"), Some((8.0, "oz".to_string())));
        assert_eq!(parse_quantity("1 bar"), Some((1.0, "bar".to_string())));
    }

    #[test]
    fn test_to_grams() {
        assert_eq!(to_grams(100.0, "g"), Some(100.0));
        assert!((to_grams(1.0, "oz").unwrap() - 28.3495).abs() < 0.01);
    }
}
