//! Catalog selection for background work, independent of device I/O.

use crate::config::Catalog;

pub enum Job {
    /// An explicit catalog is synced even when excluded from sync-all.
    Sync(Option<String>),
    CollectionsOnly,
}

impl Job {
    pub fn catalogs(&self, catalogs: Vec<Catalog>) -> Result<Vec<Catalog>, String> {
        match self {
            Self::Sync(Some(id)) => catalogs
                .into_iter()
                .find(|c| c.id == *id)
                .map(|catalog| vec![catalog])
                .ok_or_else(|| format!("catalog not found: {}", id)),
            Self::Sync(None) => {
                let enabled: Vec<_> = catalogs.into_iter().filter(|c| c.enabled).collect();
                if enabled.is_empty() {
                    Err("no enabled catalogs to sync".to_string())
                } else {
                    Ok(enabled)
                }
            }
            Self::CollectionsOnly => Ok(catalogs),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalogs() -> Vec<Catalog> {
        serde_json::from_str(
            r#"[
            {"id":"first","name":"First","url":"https://first.test","enabled":true},
            {"id":"disabled","name":"Disabled","url":"https://disabled.test","enabled":false},
            {"id":"last","name":"Last","url":"https://last.test","enabled":true}
        ]"#,
        )
        .unwrap()
    }

    #[test]
    fn sync_all_filters_disabled_catalogs_and_preserves_order() {
        let selected = Job::Sync(None).catalogs(catalogs()).unwrap();
        assert_eq!(
            selected.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["first", "last"]
        );
        assert_eq!(
            Job::Sync(None).catalogs(Vec::new()).unwrap_err(),
            "no enabled catalogs to sync"
        );
        assert!(Job::Sync(None)
            .catalogs(vec![catalogs()[1].clone()])
            .is_err());
    }

    #[test]
    fn explicit_sync_includes_disabled_catalog_and_reports_missing_id() {
        let selected = Job::Sync(Some("disabled".into()))
            .catalogs(catalogs())
            .unwrap();
        assert_eq!(selected, catalogs()[1..2]);
        assert_eq!(
            Job::Sync(Some("missing".into()))
                .catalogs(catalogs())
                .unwrap_err(),
            "catalog not found: missing"
        );
    }

    #[test]
    fn collections_rebuild_includes_every_catalog() {
        assert_eq!(
            Job::CollectionsOnly.catalogs(catalogs()).unwrap(),
            catalogs()
        );
        assert!(Job::CollectionsOnly
            .catalogs(Vec::new())
            .unwrap()
            .is_empty());
    }
}
