//! `Routing` — which agent provider works which role (cost control). Reads
//! and writes the `routing.<role>` settings; validation is the whole job:
//! unknown roles and unknown providers never reach the database.
//!
//! Concrete `Store`, like the other store-shaped use cases.

use super::UseCaseError;
use crate::store::Store;

/// The fixed team (the role table's seeded ids).
pub const ROLES: [&str; 5] = ["manager", "architect", "backend", "frontend", "reviewer"];

/// Providers the channel knows how to spawn.
pub const PROVIDERS: [&str; 2] = ["claude", "codex"];

/// One role's assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleRouting {
    pub role: String,
    pub provider: String,
}

pub struct Routing {
    store: Store,
}

impl Routing {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// Every role's provider; a missing row means claude (the default).
    pub async fn get(&self) -> Result<Vec<RoleRouting>, UseCaseError> {
        let mut out = Vec::new();
        for role in ROLES {
            let provider = self
                .store
                .get_setting(&format!("routing.{role}"))
                .await?
                .unwrap_or_else(|| "claude".into());
            out.push(RoleRouting {
                role: role.into(),
                provider,
            });
        }
        Ok(out)
    }

    /// Replace assignments. All-or-nothing: validate everything first.
    /// Returns the providers that actually changed, so the caller can evict
    /// the live sessions they affect.
    pub async fn set(&self, entries: &[RoleRouting]) -> Result<Vec<String>, UseCaseError> {
        for entry in entries {
            if !ROLES.contains(&entry.role.as_str()) {
                return Err(UseCaseError::NotFound("role"));
            }
            if !PROVIDERS.contains(&entry.provider.as_str()) {
                return Err(UseCaseError::Domain(
                    latoile_core::error::DomainError::Invariant("provider must be claude or codex"),
                ));
            }
        }
        let current = self.get().await?;
        let mut changed = Vec::new();
        for entry in entries {
            let before = current
                .iter()
                .find(|c| c.role == entry.role)
                .map(|c| c.provider.as_str());
            if before != Some(entry.provider.as_str()) {
                self.store
                    .set_setting(&format!("routing.{}", entry.role), &entry.provider)
                    .await?;
                changed.push(entry.role.clone());
            }
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn get_returns_the_seeded_defaults() {
        let store = Store::open_ephemeral().await.unwrap();
        let routing = Routing::new(store).get().await.unwrap();
        assert_eq!(routing.len(), 5);
        assert!(routing.iter().all(|r| r.provider == "claude"));
    }

    #[tokio::test]
    async fn set_persists_and_reports_what_changed() {
        let store = Store::open_ephemeral().await.unwrap();
        let routing = Routing::new(store.clone());
        let changed = routing
            .set(&[
                RoleRouting {
                    role: "backend".into(),
                    provider: "codex".into(),
                },
                RoleRouting {
                    role: "manager".into(),
                    provider: "claude".into(), // unchanged
                },
            ])
            .await
            .unwrap();
        assert_eq!(changed, ["backend"]);
        let after = routing.get().await.unwrap();
        assert_eq!(
            after.iter().find(|r| r.role == "backend").unwrap().provider,
            "codex"
        );
        assert_eq!(
            after.iter().find(|r| r.role == "manager").unwrap().provider,
            "claude"
        );
    }

    #[tokio::test]
    async fn unknown_providers_and_roles_are_refused() {
        let store = Store::open_ephemeral().await.unwrap();
        let routing = Routing::new(store);
        assert!(routing
            .set(&[RoleRouting {
                role: "backend".into(),
                provider: "gemini".into(),
            }])
            .await
            .is_err());
        assert!(routing
            .set(&[RoleRouting {
                role: "poet".into(),
                provider: "claude".into(),
            }])
            .await
            .is_err());
    }
}
