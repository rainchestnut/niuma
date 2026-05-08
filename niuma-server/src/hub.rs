//! In-memory WebSocket connection hub for authenticated mobile and gateway peers.

use std::{collections::HashMap, time::Duration};

use anyhow::{Result, bail};
use axum::extract::ws::Message;
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::crypto;

#[derive(Debug, Clone)]
struct MobileConnection {
    agent_id: String,
    connection_id: String,
    sender: mpsc::UnboundedSender<Message>,
}

#[derive(Debug, Clone)]
struct AgentConnection {
    connection_id: String,
    sender: mpsc::UnboundedSender<Message>,
}

#[derive(Default)]
pub struct ConnectionHub {
    mobile: Mutex<HashMap<String, MobileConnection>>,
    agents: Mutex<HashMap<String, AgentConnection>>,
    pair_waiters: Mutex<HashMap<String, oneshot::Sender<Value>>>,
}

impl ConnectionHub {
    /// Register one active mobile socket, replacing any previous socket for the device.
    pub async fn connect_mobile(
        &self,
        device_id: &str,
        agent_id: &str,
        sender: mpsc::UnboundedSender<Message>,
    ) -> String {
        let connection_id = crypto::random_token(9);
        let old = self.mobile.lock().await.insert(
            device_id.to_string(),
            MobileConnection {
                agent_id: agent_id.to_string(),
                connection_id: connection_id.clone(),
                sender,
            },
        );
        if let Some(old) = old {
            let _ = old.sender.send(Message::Close(None));
        }
        connection_id
    }

    /// Register one active gateway socket, replacing any previous socket for the agent.
    pub async fn connect_agent(
        &self,
        agent_id: &str,
        sender: mpsc::UnboundedSender<Message>,
    ) -> String {
        let connection_id = crypto::random_token(9);
        let old = self.agents.lock().await.insert(
            agent_id.to_string(),
            AgentConnection {
                connection_id: connection_id.clone(),
                sender,
            },
        );
        if let Some(old) = old {
            let _ = old.sender.send(Message::Close(None));
        }
        connection_id
    }

    pub async fn disconnect_mobile(&self, device_id: &str, connection_id: &str) {
        let mut mobile = self.mobile.lock().await;
        if mobile
            .get(device_id)
            .is_some_and(|connection| connection.connection_id == connection_id)
        {
            mobile.remove(device_id);
        }
    }

    pub async fn disconnect_agent(&self, agent_id: &str, connection_id: &str) {
        let mut agents = self.agents.lock().await;
        if agents
            .get(agent_id)
            .is_some_and(|connection| connection.connection_id == connection_id)
        {
            agents.remove(agent_id);
        }
    }

    pub async fn connected_mobile_ids_for_agent(&self, agent_id: &str) -> Vec<String> {
        let mut ids: Vec<_> = self
            .mobile
            .lock()
            .await
            .iter()
            .filter_map(|(device_id, connection)| {
                (connection.agent_id == agent_id).then_some(device_id.clone())
            })
            .collect();
        ids.sort();
        ids
    }

    pub async fn send_to_mobile(
        &self,
        device_id: &str,
        payload: Value,
        agent_id: Option<&str>,
    ) -> bool {
        let connection = self.mobile.lock().await.get(device_id).cloned();
        let Some(connection) = connection else {
            return false;
        };
        if agent_id.is_some_and(|expected| expected != connection.agent_id) {
            return false;
        }
        send_json(&connection.sender, payload)
    }

    pub async fn send_to_agent(&self, agent_id: &str, payload: Value) -> bool {
        let connection = self.agents.lock().await.get(agent_id).cloned();
        let Some(connection) = connection else {
            return false;
        };
        send_json(&connection.sender, payload)
    }

    pub async fn request_agent_pair_handshake(
        &self,
        agent_id: &str,
        mut payload: Value,
    ) -> Result<Value> {
        let request_id = crypto::random_token(12);
        let (tx, rx) = oneshot::channel();
        self.pair_waiters
            .lock()
            .await
            .insert(request_id.clone(), tx);
        payload["kind"] = json!("pair_handshake");
        payload["request_id"] = json!(request_id.clone());
        if !self.send_to_agent(agent_id, payload).await {
            self.pair_waiters.lock().await.remove(&request_id);
            bail!("desktop gateway is offline");
        }
        let result = tokio::time::timeout(Duration::from_secs(10), rx).await;
        self.pair_waiters.lock().await.remove(&request_id);
        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => bail!("desktop gateway pair handshake failed"),
            Err(_) => bail!("desktop gateway pair handshake timed out"),
        }
    }

    pub async fn complete_agent_pair_handshake(&self, agent_id: &str, payload: &Value) -> bool {
        let Some(request_id) = payload.get("request_id").and_then(Value::as_str) else {
            return false;
        };
        let Some(sender) = self.pair_waiters.lock().await.remove(request_id) else {
            return false;
        };
        let mut payload = payload.clone();
        if payload.get("agent_id").and_then(Value::as_str) != Some(agent_id) {
            payload["ack_status"] = json!("rejected");
            payload["error"] = json!("pair handshake ack agent mismatch");
        }
        sender.send(payload).is_ok()
    }
}

fn send_json(sender: &mpsc::UnboundedSender<Message>, payload: Value) -> bool {
    match serde_json::to_string(&payload) {
        Ok(text) => sender.send(Message::Text(text.into())).is_ok(),
        Err(_) => false,
    }
}
