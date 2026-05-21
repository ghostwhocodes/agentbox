//! Shared kernel and cross-context utilities.
//!
//! This module holds validated identifiers, persistence contracts, shared
//! path/error plumbing, and low-level filesystem helpers reused across
//! bounded contexts.

pub mod error;
pub(crate) mod fs_ops;
pub mod mount_spec;
pub(crate) mod paths;
pub mod types;
