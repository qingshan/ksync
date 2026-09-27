//! Status snapshot shared with the WAF (written to status.json and served via
//! the LIPC `status` property). Exact camelCase schema documented in the
//! package README; `catalogs` is a password-free config snapshot so the WAF
//! renders the catalog list from status.json alone.

use serde::{Deserialize, Serialize};

pub const STATE_IDLE: &str = "idle";
pub const STATE_RUNNING: &str = "running";
pub const STATE_STOPPING: &str = "stopping";
pub const STATE_DONE: &str = "done";
pub const STATE_ERROR: &str = "error";

pub const PHASE_IDLE: &str = "idle";
pub const PHASE_SYNCING: &str = "syncing";
pub const PHASE_COLLECTIONS: &str = "collections";
pub const PHASE_DONE: &str = "done";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSummary {
    pub id: String,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub insecure: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: String,
    pub catalog_id: Option<String>,
    pub catalog_name: Option<String>,
    pub phase: String,
    pub downloaded: u64,
    pub skipped: u64,
    pub failed: u64,
    pub current: String,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    pub last_error: Option<String>,
    pub collections_added: u64,
    pub collections_pending: u64,
    pub collections_error: Option<String>,
    pub updated_at: u64,
    /// Global Library collection prefix applied to every catalog
    /// (`<prefix> <catalog name>`), from `var/ksync.json`.
    pub collection_prefix: String,
    pub catalogs: Vec<CatalogSummary>,
}

impl Default for Status {
    fn default() -> Self {
        Status {
            state: STATE_IDLE.to_string(),
            catalog_id: None,
            catalog_name: None,
            phase: PHASE_IDLE.to_string(),
            downloaded: 0,
            skipped: 0,
            failed: 0,
            current: String::new(),
            started_at: None,
            finished_at: None,
            last_error: None,
            collections_added: 0,
            collections_pending: 0,
            collections_error: None,
            updated_at: 0,
            collection_prefix: crate::config::default_collection_prefix(),
            catalogs: Vec::new(),
        }
    }
}

impl Status {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::field_reassign_with_default)] // test fixture built field-by-field
    fn serializes_to_exact_camel_case_schema() {
        let mut s = Status::default();
        s.state = STATE_RUNNING.to_string();
        s.catalog_id = Some("gutenberg".to_string());
        s.phase = PHASE_SYNCING.to_string();
        s.downloaded = 2;
        s.catalogs.push(CatalogSummary {
            id: "gutenberg".into(),
            name: "Gutenberg".into(),
            url: "https://www.gutenberg.org/ebooks.opds/".into(),
            enabled: true,
            insecure: false,
        });
        let json = s.to_json();
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let obj = v.as_object().expect("object");
        let expected_keys = [
            "state",
            "catalogId",
            "catalogName",
            "phase",
            "downloaded",
            "skipped",
            "failed",
            "current",
            "startedAt",
            "finishedAt",
            "lastError",
            "collectionsAdded",
            "collectionsPending",
            "collectionsError",
            "updatedAt",
            "collectionPrefix",
            "catalogs",
        ];
        for k in expected_keys {
            assert!(obj.contains_key(k), "missing key {k} in {}", json);
        }
        assert_eq!(obj["state"], "running");
        assert_eq!(obj["catalogId"], "gutenberg");
        assert_eq!(obj["downloaded"], 2);
        let cats = obj["catalogs"].as_array().expect("array");
        assert_eq!(cats.len(), 1);
        let c0 = cats[0].as_object().expect("object");
        assert!(
            !c0.contains_key("password"),
            "passwords must never be serialized"
        );
        assert!(c0.contains_key("insecure"));
        // round-trips through the same struct (Deserialize derives match)
        let back: Status = serde_json::from_str(&json).expect("round-trip");
        assert_eq!(back.state, STATE_RUNNING);
        assert_eq!(obj["collectionPrefix"], "KSync");
    }
}
