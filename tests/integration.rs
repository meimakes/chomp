use chomp::db::Database;
use chomp::food::Food;
use chomp::logging::parse_and_log;

#[test]
fn test_full_workflow() {
    let db = Database::open_in_memory().unwrap();

    // Add a food
    let food = Food::new("Ribeye", 26.0, 15.0, 0.0, 250.0, "100g", vec!["steak".to_string()]);
    let food_id = db.add_food(&food).unwrap();
    assert!(food_id > 0);

    // Log it via parse_and_log
    let entry = parse_and_log(&db, "ribeye 8oz").unwrap();
    assert_eq!(entry.food_name, "Ribeye");
    assert!(entry.calories > 0.0);

    // Check today's totals
    let totals = db.get_today_totals().unwrap();
    assert!(totals.calories > 0.0);
    assert_eq!(totals.protein, entry.protein);

    // Check history
    let history = db.get_history(7).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].food_name, "Ribeye");

    // Log via alias
    let entry2 = parse_and_log(&db, "steak 200g").unwrap();
    assert_eq!(entry2.food_name, "Ribeye");

    // Totals should have both
    let totals = db.get_today_totals().unwrap();
    assert_eq!(totals.protein, entry.protein + entry2.protein);
}

#[test]
fn test_food_not_found() {
    let db = Database::open_in_memory().unwrap();
    let result = parse_and_log(&db, "nonexistent 100g");
    assert!(result.is_err());
}
