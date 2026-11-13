//! Headless database services for DBPinball.
//!
//! This crate must not depend on Tauri, a CLI parser, or terminal I/O. The
//! desktop application and the future CLI are adapters over these services.

pub mod adapters;
pub mod domain;
pub mod ports;
pub mod services;
pub mod store;
