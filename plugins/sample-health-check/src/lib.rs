//! Reference implementation of `gfc:health-check@1.0.0`.
//! Compiled to native for tests; the same logic is what a WASM guest would run.

use gfc_schema::{HealthThresholds, Inventory};

pub fn check(inventory_json: &str) -> String {
    let parsed: Result<Inventory, _> = serde_json::from_str(inventory_json);
    match parsed {
        Ok(inv) => {
            let n = inv.attention_count(&HealthThresholds::default());
            format!("{{\"attention\":{n},\"repos\":{}}}", inv.repositories.len())
        }
        Err(err) => format!("{{\"error\":\"{err}\"}}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_inventory() {
        let out = check(r#"{"schema_version":"1.0.0","generated_at":"2026-08-20T00:00:00Z","repositories":[],"errors":[]}"#);
        assert!(out.contains("\"attention\":0"));
    }
}
