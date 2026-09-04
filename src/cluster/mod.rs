//! Cluster roles, node state, and wire protocol.

mod protocol;

pub use protocol::{ControlMessage, PairDecision, PairRequest, PairResponse};

use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    #[default]
    Automatic,
    Main,
    Worker,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    #[default]
    Discovered,
    Pairing,
    Trusted,
    Ready,
    Busy,
    Offline,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSummary {
    pub id: Uuid,
    pub name: String,
    pub role: NodeRole,
    pub status: NodeStatus,
    pub addresses: Vec<IpAddr>,
    pub control_port: u16,
    pub rpc_port: u16,
    pub available_memory_bytes: u64,
    pub compute_backend: String,
}

impl NodeSummary {
    pub fn is_usable_worker(&self) -> bool {
        matches!(self.status, NodeStatus::Trusted | NodeStatus::Ready)
            && !matches!(self.role, NodeRole::Main)
            && self.available_memory_bytes > 0
    }
}
