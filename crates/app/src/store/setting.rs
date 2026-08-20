//! `setting` table — key/value rows. No core entity: settings are adapter
//! configuration, not domain. First citizen: `routing.<role>` → provider.

use super::{Store, StoreError};
use latoile_core::ports::PortResult;
use sqlx::Row;

impl Store {
    pub async fn get_setting(&self, key: &str) -> PortResult<Option<String>> {
        let row = sqlx::query("SELECT value FROM setting WHERE key = ?")
            .bind(key)
            .fetch_optional(self.pool())
            .await
            .map_err(StoreError::from)?;
        Ok(row.map(|r| r.get("value")))
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> PortResult<()> {
        sqlx::query(
            "INSERT INTO setting (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn settings_round_trip_and_overwrite() {
        let store = Store::open_ephemeral().await.unwrap();
        assert!(store
            .get_setting("routing.manager")
            .await
            .unwrap()
            .is_some());
        assert!(store.get_setting("nope").await.unwrap().is_none());

        store.set_setting("routing.manager", "codex").await.unwrap();
        assert_eq!(
            store
                .get_setting("routing.manager")
                .await
                .unwrap()
                .as_deref(),
            Some("codex")
        );
    }

    #[tokio::test]
    async fn the_seeded_routing_defaults_to_claude() {
        let store = Store::open_ephemeral().await.unwrap();
        for role in ["manager", "architect", "backend", "frontend", "reviewer"] {
            assert_eq!(
                store
                    .get_setting(&format!("routing.{role}"))
                    .await
                    .unwrap()
                    .as_deref(),
                Some("claude"),
                "{role}"
            );
        }
    }
}
