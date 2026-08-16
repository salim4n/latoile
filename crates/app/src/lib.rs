//! Use cases. One file per use case in `use_cases/`: SendMessage, DispatchTask,
//! GrantApproval, EnsurePreview… Each follows the same shape — validate input,
//! load entities, call domain methods, persist, emit events, return a DTO.
//!
//! Orchestrates through the ports defined in `latoile-core`; knows neither
//! axum nor the outside world. The persistence module (`store/`) is the only
//! place SQL lives outside the vault.

pub mod manager_actions;
pub mod store;
pub mod supervision;
pub mod use_cases;
