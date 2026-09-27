//! JSON commands and in-memory catalog edits.

use crate::config::{self, Catalog};

/// Commands accepted via the `cmd` property (serde tag `op`, snake_case names).
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    CatalogAdd(CatalogInput),
    CatalogUpdate {
        id: String,
        #[serde(flatten)]
        catalog: CatalogInput,
    },
    CatalogRemove {
        id: String,
    },
    CatalogSetEnabled {
        id: String,
        enabled: bool,
    },
    SyncStart {
        id: String,
    },
    SyncAll,
    SyncStop,
    CollectionsRebuild,
    SetCollectionPrefix {
        prefix: String,
    },
    CollectionsRemove {
        name: String,
    },
}

impl Op {
    /// A safe command-history value for the public LIPC getter.
    pub fn name(&self) -> &'static str {
        match self {
            Op::CatalogAdd(..) => "catalog_add",
            Op::CatalogUpdate { .. } => "catalog_update",
            Op::CatalogRemove { .. } => "catalog_remove",
            Op::CatalogSetEnabled { .. } => "catalog_set_enabled",
            Op::SyncStart { .. } => "sync_start",
            Op::SyncAll => "sync_all",
            Op::SyncStop => "sync_stop",
            Op::CollectionsRebuild => "collections_rebuild",
            Op::SetCollectionPrefix { .. } => "set_collection_prefix",
            Op::CollectionsRemove { .. } => "collections_remove",
        }
    }
}

/// Editable catalog fields; empty update credentials preserve stored values.
#[derive(Debug, serde::Deserialize)]
pub struct CatalogInput {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub insecure: bool,
    #[serde(default)]
    pub enabled: bool,
}

impl CatalogInput {
    pub fn add_to(self, catalogs: &mut Vec<Catalog>) -> String {
        let ids: Vec<_> = catalogs.iter().map(|c| c.id.clone()).collect();
        let id = config::catalog_id_for(&self.name, &ids);
        catalogs.push(Catalog {
            id: id.clone(),
            name: self.name,
            url: self.url,
            username: self.username,
            password: self.password,
            insecure: self.insecure,
            enabled: self.enabled,
        });
        id
    }

    pub fn update_in(self, id: &str, catalogs: &mut [Catalog]) -> Result<(), String> {
        let catalog = find_catalog_mut(id, catalogs)?;
        catalog.name = self.name;
        catalog.url = self.url;
        catalog.insecure = self.insecure;
        catalog.enabled = self.enabled;
        if let Some(username) = self.username.filter(|s| !s.is_empty()) {
            catalog.username = Some(username);
        }
        if let Some(password) = self.password.filter(|s| !s.is_empty()) {
            catalog.password = Some(password);
        }
        Ok(())
    }
}

pub fn remove_catalog(id: &str, catalogs: &mut Vec<Catalog>) -> Result<(), String> {
    find_catalog_mut(id, catalogs)?;
    catalogs.retain(|c| c.id != id);
    Ok(())
}

pub fn set_catalog_enabled(
    id: &str,
    enabled: bool,
    catalogs: &mut [Catalog],
) -> Result<(), String> {
    find_catalog_mut(id, catalogs)?.enabled = enabled;
    Ok(())
}

fn find_catalog_mut<'a>(id: &str, catalogs: &'a mut [Catalog]) -> Result<&'a mut Catalog, String> {
    catalogs
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("catalog not found: {}", id))
}
#[cfg(test)]
mod tests {
    use super::*;

    fn input(json: &str) -> CatalogInput {
        serde_json::from_str(json).unwrap()
    }

    fn catalogs() -> Vec<Catalog> {
        let mut catalogs = Vec::new();
        input(r#"{"name":"Books","url":"https://example.test","username":"reader","password":"secret","enabled":true}"#)
            .add_to(&mut catalogs);
        catalogs
    }

    #[test]
    fn accepts_existing_flat_command_protocol() {
        let commands = [
            (
                r#"{"op":"catalog_add","name":"Books","url":"https://example.test"}"#,
                "catalog_add",
            ),
            (
                r#"{"op":"catalog_update","id":"Books","name":"New","url":"https://example.test","enabled":true}"#,
                "catalog_update",
            ),
            (r#"{"op":"catalog_remove","id":"Books"}"#, "catalog_remove"),
            (
                r#"{"op":"catalog_set_enabled","id":"Books","enabled":false}"#,
                "catalog_set_enabled",
            ),
            (r#"{"op":"sync_start","id":"Books"}"#, "sync_start"),
            (r#"{"op":"sync_all"}"#, "sync_all"),
            (r#"{"op":"sync_stop"}"#, "sync_stop"),
            (r#"{"op":"collections_rebuild"}"#, "collections_rebuild"),
            (
                r#"{"op":"set_collection_prefix","prefix":"Library"}"#,
                "set_collection_prefix",
            ),
            (
                r#"{"op":"collections_remove","name":"Library Books"}"#,
                "collections_remove",
            ),
        ];
        for (json, name) in commands {
            let op: Op = serde_json::from_str(json).unwrap();
            assert_eq!(op.name(), name);
        }
        for invalid in [
            r#"{"op":"unknown"}"#,
            r#"{"op":"catalog_add","name":"Books"}"#,
            r#"{"op":"catalog_update","name":"Books","url":"https://example.test"}"#,
        ] {
            assert!(serde_json::from_str::<Op>(invalid).is_err());
        }
        let Op::CatalogAdd(catalog) = serde_json::from_str(commands[0].0).unwrap() else {
            panic!("expected catalog_add");
        };
        assert!(!catalog.enabled);
        assert!(!catalog.insecure);
        assert!(catalog.username.is_none());
    }

    #[test]
    fn edits_preserve_id_and_unprovided_credentials() {
        let mut catalogs = catalogs();
        for credentials in ["", r#", "username":"", "password":"""#] {
            let json = format!(
                r#"{{"name":"Renamed","url":"https://new.test","insecure":true{credentials}}}"#
            );
            input(&json).update_in("Books", &mut catalogs).unwrap();
            let catalog = &catalogs[0];
            assert_eq!(catalog.id, "Books");
            assert_eq!(catalog.name, "Renamed");
            assert_eq!(catalog.url, "https://new.test");
            assert!(catalog.insecure);
            assert!(!catalog.enabled);
            assert_eq!(catalog.username.as_deref(), Some("reader"));
            assert_eq!(catalog.password.as_deref(), Some("secret"));
        }
        input(r#"{"name":"Books","url":"https://example.test","username":"new","password":"replacement"}"#)
            .update_in("Books", &mut catalogs).unwrap();
        assert_eq!(catalogs[0].username.as_deref(), Some("new"));
        assert_eq!(catalogs[0].password.as_deref(), Some("replacement"));
    }

    #[test]
    fn catalog_mutations_target_only_the_requested_id() {
        let mut catalogs = catalogs();
        let id = input(r#"{"name":"Books","url":"https://second.test"}"#).add_to(&mut catalogs);
        assert_eq!(id, "Books-2");
        let original = catalogs.clone();
        assert_eq!(
            set_catalog_enabled("missing", true, &mut catalogs).unwrap_err(),
            "catalog not found: missing"
        );
        assert!(remove_catalog("missing", &mut catalogs).is_err());
        assert!(input(r#"{"name":"New","url":"https://new.test"}"#)
            .update_in("missing", &mut catalogs)
            .is_err());
        assert_eq!(catalogs, original);
        set_catalog_enabled(&id, true, &mut catalogs).unwrap();
        assert!(catalogs[1].enabled);
        assert_eq!(catalogs[0], original[0]);
        remove_catalog(&id, &mut catalogs).unwrap();
        assert_eq!(catalogs, original[..1]);
    }
}
