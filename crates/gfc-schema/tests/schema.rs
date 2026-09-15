use gfc_schema::{json_schema, Inventory, SCHEMA_ID, SCHEMA_VERSION};
use serde_json::json;

#[test]
fn inventory_round_trips() {
    let now = chrono::Utc::now();
    let raw = json!({
        "schema_version": SCHEMA_VERSION,
        "generated_at": now,
        "repositories": [],
        "errors": []
    });
    let inventory: Inventory = serde_json::from_value(raw.clone()).unwrap();
    let back = serde_json::to_value(&inventory).unwrap();
    assert_eq!(back["schema_version"], SCHEMA_VERSION);
    assert_eq!(inventory.repositories.len(), 0);
}

#[test]
fn published_schema_is_valid_json_schema() {
    let schema = json_schema();
    let value = serde_json::to_value(&schema).expect("schema serializes");
    assert!(value.get("$schema").is_some() || value.get("title").is_some() || value.get("definitions").is_some() || value.get("$ref").is_some());
    let on_disk = include_str!("../../../schema/repository-health.v1.json");
    let disk: serde_json::Value = serde_json::from_str(on_disk).unwrap();
    assert_eq!(disk["$id"], SCHEMA_ID);
    assert_eq!(disk["version"], SCHEMA_VERSION);
}

#[test]
fn golden_repository_deserializes() {
    let golden = include_str!("golden_repository.json");
    let repo: gfc_schema::RepositoryHealth = serde_json::from_str(golden).unwrap();
    assert_eq!(repo.schema_version, SCHEMA_VERSION);
    assert_eq!(repo.identity.name, "demo");
}
