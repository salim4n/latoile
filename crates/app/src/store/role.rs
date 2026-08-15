//! `role` table reads. Roles have no core entity — the domain treats a
//! `RoleId` as opaque — so this returns plain rows for the `/api/roles`
//! route, which only ever displays them.

use super::{Store, StoreError};
use latoile_core::ports::PortResult;
use sqlx::Row;

/// A role as the roles screen shows it. Not a domain type: a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleRow {
    pub id: String,
    pub label: String,
    pub skill_path: Option<String>,
    pub cli: String,
    pub system_prompt_path: Option<String>,
}

impl Store {
    pub async fn list_roles(&self) -> PortResult<Vec<RoleRow>> {
        let rows = sqlx::query(
            "SELECT id, label, skill_path, cli, system_prompt_path FROM role ORDER BY id",
        )
        .fetch_all(self.pool())
        .await
        .map_err(StoreError::from)?;
        Ok(rows
            .iter()
            .map(|r| RoleRow {
                id: r.get("id"),
                label: r.get("label"),
                skill_path: r.get("skill_path"),
                cli: r.get("cli"),
                system_prompt_path: r.get("system_prompt_path"),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_seeded_team_is_listed() {
        let store = Store::open_ephemeral().await.unwrap();
        let roles = store.list_roles().await.unwrap();
        let ids: Vec<&str> = roles.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["architect", "backend", "frontend", "manager", "reviewer"]);
    }
}
