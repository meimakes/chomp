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
    
    // If amount is unitless (defaulted to "g") but serving is a discrete unit,
    // treat the amount as that discrete unit instead of grams.
    // e.g., "2" with serving "1piece" means 2 pieces, not 2 grams.
    let discrete_units = ["bar", "bars", "piece", "pieces", "serving", "servings", "scoop", "scoops", "slice", "slices", "patty", "patties", "pack", "packs"];
    if amount_unit == "g" && amount.trim().parse::<f64>().is_ok() && discrete_units.contains(&serving_unit.as_str()) {
        return Some(amount_val / serving_val);
    }
    
    // Convert both to grams for comparison
    let amount_grams = to_grams(amount_val, &amount_unit)?;
    let serving_grams = to_grams(serving_val, &serving_unit)?;
    
    Some(amount_grams / serving_grams)
}

fn parse_quantity(s: &str) -> Option<(f64, String)> {
    let s = s.trim().to_lowercase();
    
    // Split by whitespace first to handle "4 oz", "1 bar", etc.
    let parts: Vec<&str> = s.split_whitespace().collect();
    
    if parts.len() == 2 {
        // "4 oz" pattern
        let num: f64 = parts[0].parse().ok()?;
        let unit = parts[1].to_string();
        Some((num, unit))
    } else if parts.len() == 1 {
        // Could be "4oz" or just "4"
        let part = parts[0];
        if let Some(num_end) = part.find(|c: char| !c.is_numeric() && c != '.') {
            let num_str = &part[..num_end];
            let unit = part[num_end..].to_string();
            let num: f64 = num_str.parse().ok()?;
            Some((num, unit))
        } else {
            // Just a number, assume grams
            let num: f64 = part.parse().ok()?;
            Some((num, "g".to_string()))
        }
    } else {
        None
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
        "bar" | "bars" | "piece" | "pieces" | "serving" | "servings" | "scoop" | "scoops" | "slice" | "slices" | "patty" | "patties" | "pack" | "packs" => Some(value * 100.0),
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
        assert_eq!(parse_quantity("4 oz"), Some((4.0, "oz".to_string())));
        assert_eq!(parse_quantity("0.5 oz"), Some((0.5, "oz".to_string())));
        assert_eq!(parse_quantity("3 patties"), Some((3.0, "patties".to_string())));
        assert_eq!(parse_quantity("2 packs"), Some((2.0, "packs".to_string())));
    }

    #[test]
    fn test_to_grams() {
        assert_eq!(to_grams(100.0, "g"), Some(100.0));
        assert!((to_grams(1.0, "oz").unwrap() - 28.3495).abs() < 0.01);
        assert!((to_grams(1.0, "lb").unwrap() - 453.592).abs() < 0.01);
        assert_eq!(to_grams(1.0, "kg"), Some(1000.0));
        assert_eq!(to_grams(1.0, "cup"), Some(240.0));
        assert_eq!(to_grams(1.0, "tbsp"), Some(15.0));
        assert_eq!(to_grams(1.0, "tsp"), Some(5.0));
        assert_eq!(to_grams(1.0, "bar"), Some(100.0));
    }

    #[test]
    fn test_calculate_same_unit() {
        let food = Food::new("Rice", 2.7, 0.3, 28.0, 130.0, "100g", vec![]);
        let m = food.calculate("200g").unwrap();
        assert!((m.protein - 5.4).abs() < 0.01);
        assert!((m.calories - 260.0).abs() < 0.01);
    }

    #[test]
    fn test_calculate_cross_unit() {
        // 8oz of a food with 100g serving
        let food = Food::new("Ribeye", 26.0, 15.0, 0.0, 250.0, "100g", vec![]);
        let m = food.calculate("8oz").unwrap();
        let expected_mult = (8.0 * 28.3495) / 100.0;
        assert!((m.protein - 26.0 * expected_mult).abs() < 0.1);
    }

    #[test]
    fn test_calculate_serving_unit() {
        let food = Food::new("Bare Bar", 20.0, 7.0, 22.0, 210.0, "1bar", vec![]);
        let m = food.calculate("1bar").unwrap();
        assert!((m.calories - 210.0).abs() < 0.01);
    }

    #[test]
    fn test_macros_add() {
        let mut a = Macros { protein: 10.0, fat: 5.0, carbs: 20.0, calories: 165.0 };
        let b = Macros { protein: 5.0, fat: 3.0, carbs: 10.0, calories: 87.0 };
        a.add(&b);
        assert_eq!(a.protein, 15.0);
        assert_eq!(a.calories, 252.0);
    }
}
