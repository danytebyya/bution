use super::NodeSummary;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairRequest {
    pub node_id: Uuid,
    pub node_name: String,
    pub public_key: String,
    pub protocol_version: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairDecision {
    Accept,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairResponse {
    pub node_id: Uuid,
    pub public_key: String,
    pub decision: PairDecision,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ControlMessage {
    PairRequest(PairRequest),
    PairResponse(PairResponse),
    NodeInfo(NodeSummary),
    GetNodeInfo,
    Ping {
        nonce: u64,
    },
    Pong {
        nonce: u64,
    },
    StartWorker {
        bind_address: String,
        rpc_port: u16,
    },
    StopWorker,
    StartNetworkBenchmark {
        bind_address: String,
        port: u16,
    },
    StopNetworkBenchmark,
    NetworkBenchmarkReady {
        port: u16,
    },
    WorkerReady {
        rpc_port: u16,
    },
    Error {
        message: String,
        detail: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_message_round_trips_through_json() {
        let message = ControlMessage::Ping { nonce: 42 };
        let encoded = serde_json::to_string(&message).unwrap();
        let decoded: ControlMessage = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, message);
    }
}
