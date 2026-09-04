//! Core library for the BUTION distributed local AI cluster.

pub mod cluster;
pub mod discovery;
pub mod hardware;
pub mod network;
pub mod security;
pub mod storage;

/// Human-readable application name.
pub const APP_NAME: &str = "BUTION";

/// Protocol version advertised over discovery and checked during pairing.
pub const PROTOCOL_VERSION: u16 = 1;
