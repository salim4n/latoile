//! `/api/settings/routing` — which provider works which role. The write
//! path refreshes the channel's live routing and evicts manager sessions,
//! so a change applies to the next message, never mid-conversation.

use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use latoile_app::use_cases::{RoleRouting, Routing};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct RoutingDto {
    pub manager: String,
    pub architect: String,
    pub backend: String,
    pub frontend: String,
    pub reviewer: String,
}

impl RoutingDto {
    fn from_entries(entries: &[RoleRouting]) -> Self {
        let get = |role: &str| {
            entries
                .iter()
                .find(|r| r.role == role)
                .map(|r| r.provider.clone())
                .unwrap_or_else(|| "claude".into())
        };
        Self {
            manager: get("manager"),
            architect: get("architect"),
            backend: get("backend"),
            frontend: get("frontend"),
            reviewer: get("reviewer"),
        }
    }
}

#[derive(Deserialize)]
pub struct RoutingBody {
    manager: String,
    architect: String,
    backend: String,
    frontend: String,
    reviewer: String,
}

pub async fn get_routing(State(state): State<AppState>) -> Result<Json<RoutingDto>, ApiError> {
    let entries = Routing::new(state.store.clone()).get().await?;
    Ok(Json(RoutingDto::from_entries(&entries)))
}

pub async fn put_routing(
    State(state): State<AppState>,
    Json(body): Json<RoutingBody>,
) -> Result<Json<RoutingDto>, ApiError> {
    let entries = vec![
        RoleRouting {
            role: "manager".into(),
            provider: body.manager,
        },
        RoleRouting {
            role: "architect".into(),
            provider: body.architect,
        },
        RoleRouting {
            role: "backend".into(),
            provider: body.backend,
        },
        RoleRouting {
            role: "frontend".into(),
            provider: body.frontend,
        },
        RoleRouting {
            role: "reviewer".into(),
            provider: body.reviewer,
        },
    ];
    let routing = Routing::new(state.store.clone());
    let changed = routing.set(&entries).await?;

    // New sessions read the new map immediately…
    let current = routing.get().await?;
    state.routing.set_all(
        current
            .iter()
            .map(|r| (r.role.clone(), r.provider.clone()))
            .collect(),
    );
    // …and a persistent Manager session is evicted only when its own
    // provider changed. Executor routing affects their next fresh run.
    if changed.iter().any(|role| role == "manager") {
        state.agents.evict_managers().await;
    }

    Ok(Json(RoutingDto::from_entries(&current)))
}
