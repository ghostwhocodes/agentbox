//! Persistence contracts and manifest I/O.
//!
//! This module owns the on-disk workspace manifest schema. Runtime contexts
//! should map these types into context-owned models instead of using them as
//! the shared in-memory collaboration model.

pub(crate) mod composite;
pub mod manifest;
pub mod manifest_store;
pub(crate) mod transaction;
