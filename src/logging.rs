use anyhow::{anyhow, Result};

use crate::db::{Database, LogEntry};

/// Parse input like "ribeye 8oz" or "bare bar" and log it
pub fn parse_and_log(db: &Database, input: &str) -> Result<LogEntry> {
    let (food_name, amount) = parse_input(input);

    // Look up the food
    let food = db.get_food_by_name(&food_name)?.ok_or_else(|| {
        anyhow!(
            "Food not found: '{}'. Add it with: chomp add \"{}\" --protein X --fat Y --carbs Z",
            food_name,
            food_name
        )
    })?;

    // Use provided amount, default amount, or serving size
    let actual_amount = if let Some(amt) = amount {
        amt
    } else if let Some(default) = &food.default_amount {
        default.clone()
    } else {
        food.serving.clone()
    };

    // Calculate macros
    let macros = food.calculate(&actual_amount).ok_or_else(|| {
        anyhow!(
            "Could not calculate macros for {} of {}",
            actual_amount,
            food.name
        )
    })?;

    // Log it
    let entry = db.log_food(food.id.unwrap(), &actual_amount, &macros)?;

    Ok(entry)
}

/// Parse input into food name and optional amount
/// Examples:
///   "ribeye 8oz" -> ("ribeye", Some("8oz"))
///   "bare bar" -> ("bare bar", None)
///   "salmon 4 oz" -> ("salmon", Some("4 oz"))
///   "heavy cream 50ml" -> ("heavy cream", Some("50ml"))
fn parse_input(input: &str) -> (String, Option<String>) {
    let input = input.trim();

    // Try to find an amount at the end
    // Look for patterns like "8oz", "4 oz", "100g", "50ml", "1 bar"

    let words: Vec<&str> = input.split_whitespace().collect();

    if words.is_empty() {
        return (String::new(), None);
    }

    if words.len() == 1 {
        return (words[0].to_string(), None);
    }

    // Check if last word is a unit or number+unit
    let last = words[words.len() - 1];
    let second_last = if words.len() > 1 {
        Some(words[words.len() - 2])
    } else {
        None
    };

    // Pattern: "salmon 4 oz" (number then unit)
    if let Some(sl) = second_last {
        if is_number(sl) && is_unit(last) {
            let amount = format!("{} {}", sl, last);
            let food_name = words[..words.len() - 2].join(" ");
            return (food_name, Some(amount));
        }
    }

    // Pattern: "salmon 4oz" (number+unit combined)
    if is_amount(last) {
        let food_name = words[..words.len() - 1].join(" ");
        return (food_name, Some(last.to_string()));
    }

    // Pattern: "3 oz milk" (number then unit then food)
    if words.len() >= 3 && is_number(words[0]) && is_unit(words[1]) {
        let amount = format!("{}{}", words[0], words[1]);
        let food_name = words[2..].join(" ");
        return (food_name, Some(amount));
    }

    // Pattern: "2 eggs" (number at start, but second word is not a recognized unit)
    if is_number(words[0]) && words.len() >= 2 {
        let amount = words[0].to_string();
        let food_name = words[1..].join(" ");
        return (food_name, Some(amount));
    }

    // Pattern: "Ortiz Sardines 0.5" (bare number at end, no unit)
    if is_number(last) && words.len() >= 2 {
        let amount = last.to_string();
        let food_name = words[..words.len() - 1].join(" ");
        return (food_name, Some(amount));
    }

    // No amount found, entire input is food name
    (input.to_string(), None)
}

fn is_number(s: &str) -> bool {
    s.parse::<f64>().is_ok()
}

fn is_unit(s: &str) -> bool {
    let units = [
        "g",
        "gram",
        "grams",
        "oz",
        "ounce",
        "ounces",
        "lb",
        "lbs",
        "pound",
        "pounds",
        "kg",
        "kilogram",
        "kilograms",
        "ml",
        "milliliter",
        "milliliters",
        "l",
        "liter",
        "liters",
        "cup",
        "cups",
        "tbsp",
        "tablespoon",
        "tablespoons",
        "tsp",
        "teaspoon",
        "teaspoons",
        "bar",
        "bars",
        "piece",
        "pieces",
        "serving",
        "servings",
        "scoop",
        "scoops",
        "slice",
        "slices",
        "patty",
        "patties",
        "pack",
        "packs",
    ];
    units.contains(&s.to_lowercase().as_str())
}

fn is_amount(s: &str) -> bool {
    // Check if it's a number followed by a unit, like "8oz" or "100g"
    let s = s.to_lowercase();

    for unit in ["g", "oz", "ml", "lb", "kg", "l"] {
        if s.ends_with(unit) {
            let num_part = &s[..s.len() - unit.len()];
            if num_part.parse::<f64>().is_ok() {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_input() {
        assert_eq!(
            parse_input("ribeye 8oz"),
            ("ribeye".to_string(), Some("8oz".to_string()))
        );
        assert_eq!(
            parse_input("salmon 4 oz"),
            ("salmon".to_string(), Some("4 oz".to_string()))
        );
        assert_eq!(parse_input("bare bar"), ("bare bar".to_string(), None));
        assert_eq!(
            parse_input("heavy cream 50ml"),
            ("heavy cream".to_string(), Some("50ml".to_string()))
        );
        assert_eq!(
            parse_input("2 eggs"),
            ("eggs".to_string(), Some("2".to_string()))
        );
    }

    #[test]
    fn test_parse_input_single_word() {
        let (name, amount) = parse_input("bacon");
        assert_eq!(name, "bacon");
        assert!(amount.is_none());
    }

    #[test]
    fn test_parse_input_empty() {
        let (name, amount) = parse_input("");
        assert_eq!(name, "");
        assert!(amount.is_none());
    }

    #[test]
    fn test_parse_input_number_first() {
        let (name, amount) = parse_input("3 slices pizza");
        // "3 slices" is number+unit, so name="pizza", amount=Some("3slices")
        assert_eq!(amount, Some("3slices".to_string()));
        assert_eq!(name, "pizza");
    }

    #[test]
    fn test_parse_input_with_unit_separated() {
        let (name, amount) = parse_input("chicken breast 6 oz");
        assert_eq!(name, "chicken breast");
        assert_eq!(amount, Some("6 oz".to_string()));
    }

    #[test]
    fn test_parse_input_bare_number_at_end() {
        let (name, amount) = parse_input("Ortiz Sardines 0.5");
        assert_eq!(name, "Ortiz Sardines");
        assert_eq!(amount, Some("0.5".to_string()));

        let (name, amount) = parse_input("eggs 3");
        assert_eq!(name, "eggs");
        assert_eq!(amount, Some("3".to_string()));
    }

    #[test]
    fn test_parse_input_fractional_amount() {
        let (name, amount) = parse_input("3 oz milk");
        assert_eq!(name, "milk");
        assert_eq!(amount, Some("3oz".to_string()));

        let (name, amount) = parse_input("0.5 oz aged gouda");
        assert_eq!(name, "aged gouda");
        assert_eq!(amount, Some("0.5oz".to_string()));

        let (name, amount) = parse_input("0.5 bare bar");
        assert_eq!(name, "bare bar");
        assert_eq!(amount, Some("0.5".to_string()));
    }
}
