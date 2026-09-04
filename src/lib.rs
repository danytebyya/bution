//! Core library for the BUTION distributed local AI cluster.

pub mod benchmark;
pub mod chat;
pub mod cluster;
pub mod control;
pub mod discovery;
pub mod hardware;
pub mod llama;
pub mod models;
pub mod network;
pub mod optimizer;
pub mod processes;
pub mod security;
pub mod storage;

/// Human-readable application name.
pub const APP_NAME: &str = "BUTION";

/// Protocol version advertised over discovery and checked during pairing.
pub const PROTOCOL_VERSION: u16 = 1;
