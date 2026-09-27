//! Catalog configuration persisted at `/mnt/us/ksync/var/catalogs.json`:
//! a JSON array of [`Catalog`]s. `load` is lenient (missing/corrupt file =>
//! empty list, rewritten on the next save); `save` is atomic (tmp + rename).

use crate::slugify;
use crate::status::CatalogSummary;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Global daemon settings, persisted at `var/ksync.json`. Currently just the
/// Library collection name prefix applied to every catalog
/// (`<collection_prefix> <catalog name>`; empty = catalog name only).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    #[serde(default = "default_collection_prefix")]
    pub collection_prefix: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            collection_prefix: default_collection_prefix(),
        }
    }
}

/// Default Library collection prefix.
pub fn default_collection_prefix() -> String {
    "KSync".to_string()
}

pub const SETTINGS_PATH: &str = "/mnt/us/ksync/var/ksync.json";

/// Read the global settings; a missing or corrupt file yields defaults
/// (rewritten on the next save).
pub fn load_settings() -> Settings {
    let path = Path::new(SETTINGS_PATH);
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

/// Write the global settings atomically (tmp + rename).
pub fn save_settings(settings: &Settings) -> Result<(), String> {
    let path = Path::new(SETTINGS_PATH);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {}", parent.display(), e))?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    let tmp = std::path::PathBuf::from(format!("{}.tmp", path.display()));
    fs::write(&tmp, json).map_err(|e| format!("write {}: {}", tmp.display(), e))?;
    fs::rename(&tmp, path)
        .map_err(|e| format!("rename {} -> {}: {}", tmp.display(), path.display(), e))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Catalog {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default)]
    pub insecure: bool,
    #[serde(default)]
    pub enabled: bool,
}

impl Catalog {
    /// Password-free snapshot for the WAF status.
    pub fn summary(&self) -> CatalogSummary {
        CatalogSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            url: self.url.clone(),
            enabled: self.enabled,
            insecure: self.insecure,
        }
    }

    /// Where this catalog's books land: `/mnt/us/documents/ksync/<id>`.
    pub fn base_dir(&self) -> std::path::PathBuf {
        std::path::Path::new(crate::DOCUMENTS_KSYNC).join(&self.id)
    }
}

/// Read the catalog list. A missing or corrupt file is treated as an empty
/// list (the next save rewrites it), never an error.
pub fn load(path: &Path) -> Vec<Catalog> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_else(|_| Vec::new())
}

/// Write the catalog list atomically (`path + ".tmp"`, then rename). Creates
/// the parent directory if needed.
pub fn save(path: &Path, catalogs: &[Catalog]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {}", parent.display(), e))?;
    }
    let json = serde_json::to_string_pretty(catalogs).map_err(|e| e.to_string())?;
    let tmp = std::path::PathBuf::from(format!("{}.tmp", path.display()));
    fs::write(&tmp, json).map_err(|e| format!("write {}: {}", tmp.display(), e))?;
    fs::rename(&tmp, path)
        .map_err(|e| format!("rename {} -> {}: {}", tmp.display(), path.display(), e))?;
    Ok(())
}

/// Derive a stable, slugified id for a new catalog, de-duplicating against
/// `existing` with `-2`, `-3`, ... suffixes.
pub fn catalog_id_for(name: &str, existing: &[String]) -> String {
    let base = slugify(name);
    if !existing.iter().any(|e| e == &base) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{}-{}", base, n);
        if !existing.iter().any(|e| e == &candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn save_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("ksync-config-test-{}", crate::now_epoch()));
        let path = dir.join("catalogs.json");
        let catalogs = vec![
            Catalog {
                id: "gutenberg".into(),
                name: "Gutenberg".into(),
                url: "https://www.gutenberg.org/ebooks.opds/".into(),
                username: Some("u".into()),
                password: Some("p".into()),
                insecure: false,
                enabled: true,
            },
            Catalog {
                id: "my-shelf".into(),
                name: "My Shelf".into(),
                url: "http://dir2opds.local:8080/opds".into(),
                username: None,
                password: None,
                insecure: true,
                enabled: false,
            },
        ];
        save(&path, &catalogs).expect("save");
        let loaded = load(&path);
        assert_eq!(loaded, catalogs);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_empty() {
        let path = PathBuf::from("/nonexistent/ksync/catalogs.json");
        assert!(load(&path).is_empty());
    }

    #[test]
    fn corrupt_file_is_empty() {
        let dir =
            std::env::temp_dir().join(format!("ksync-config-test-{}", crate::now_epoch() + 1));
        let path = dir.join("catalogs.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, "{not json").unwrap();
        assert!(load(&path).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn summary_excludes_password() {
        let c = Catalog {
            id: "x".into(),
            name: "X".into(),
            url: "http://x".into(),
            username: Some("u".into()),
            password: Some("secret".into()),
            insecure: false,
            enabled: true,
        };
        let s = c.summary();
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("secret"));
        assert_eq!(s.id, "x");
    }

    #[test]
    fn settings_default_and_serde_roundtrip() {
        // Default (and missing-file behaviour) is "KSync".
        assert_eq!(Settings::default().collection_prefix, "KSync");
        assert_eq!(load_settings().collection_prefix, "KSync");
        // A settings file without the field defaults; serde round-trips a
        // custom prefix.
        let d: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(d.collection_prefix, "KSync");
        let s = Settings {
            collection_prefix: "MyLib".into(),
        };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.collection_prefix, "MyLib");
    }

    #[test]
    fn catalog_id_dedup() {
        let existing = vec!["gutenberg".to_string(), "gutenberg-2".to_string()];
        assert_eq!(catalog_id_for("gutenberg", &existing), "gutenberg-3");
        assert_eq!(
            catalog_id_for("gutenberg", &["gutenberg".to_string()]),
            "gutenberg-2"
        );
        assert_eq!(catalog_id_for("My Shelf", &[]), "My Shelf");
        assert_eq!(catalog_id_for("Dune: Part 2!", &[]), "Dune_ Part 2_");
        assert_eq!(
            catalog_id_for(
                "a",
                &["a".to_string(), "a-2".to_string(), "a-3".to_string()]
            ),
            "a-4"
        );
    }
}
