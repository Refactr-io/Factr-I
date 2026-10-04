//! The introspection socket the integration tests drive the multi-client server through.
//!
//! It is bound only when `FACTR_DEBUG_CONTROL` is truthy, never in a normal run, and answers
//! four commands: `server:info`, `clients:map`, `create_session[:<dir>]` and, per session,
//! `state` and `history`. Everything else a debug socket once did (driving a TUI client,
//! swarm and job control, memory dumps, self-dev reload) is gone.

use super::{
    ServerIdentity, SessionInterruptQueues, SwarmMember, SwarmState, VersionedPlan,
    create_headless_session, persist_swarm_state_for,
};
use crate::agent::Agent;
use crate::protocol::{Request, ServerEvent, decode_request, encode_event};
use crate::provider::Provider;
use crate::transport::Stream;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, RwLock, mpsc};

type SessionAgents = Arc<RwLock<HashMap<String, Arc<Mutex<Agent>>>>>;

#[derive(Clone, Debug)]
pub(super) struct ClientConnectionInfo {
    pub(super) client_id: String,
    pub(super) session_id: String,
    pub(super) client_instance_id: Option<String>,
    pub(super) connected_at: Instant,
    pub(super) last_seen: Instant,
    pub(super) is_processing: bool,
    pub(super) current_tool_name: Option<String>,
    /// Terminal-identifying env vars captured from this client (tmux/zellij/
    /// kitty/DISPLAY/...). Used to route spawn/focus hooks to the client's
    /// terminal instead of the long-lived server's stale startup env (#405).
    pub(super) terminal_env: Vec<(String, String)>,
    pub(super) disconnect_tx: mpsc::UnboundedSender<()>,
}

/// The session `requested` names, else the global current one, else the only live one.
async fn resolve_debug_session(
    sessions: &SessionAgents,
    session_id: &Arc<RwLock<String>>,
    requested: Option<String>,
) -> Result<Arc<Mutex<Agent>>> {
    let mut target = requested;
    if target.is_none() {
        let current = session_id.read().await.clone();
        if !current.is_empty() {
            target = Some(current);
        }
    }

    let sessions_guard = sessions.read().await;
    if let Some(id) = target {
        return sessions_guard
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Unknown session_id '{}'", id));
    }

    if sessions_guard.len() == 1
        && let Some(agent) = sessions_guard.values().next()
    {
        return Ok(Arc::clone(agent));
    }

    Err(anyhow::anyhow!(
        "No active session found. Connect a client or provide session_id."
    ))
}

async fn server_info(
    sessions: &SessionAgents,
    swarm_members: &Arc<RwLock<HashMap<String, SwarmMember>>>,
    identity: &ServerIdentity,
    start: Instant,
) -> String {
    let live_session_ids: HashSet<String> = sessions.read().await.keys().cloned().collect();
    let members = swarm_members.read().await;
    let spawned_swarm_agent_count = members
        .values()
        .filter(|member| member.report_back_to_session_id.is_some())
        .filter(|member| live_session_ids.contains(&member.session_id))
        .count();
    serde_json::json!({
        "id": identity.id,
        "name": identity.name,
        "icon": identity.icon,
        "version": identity.version,
        "git_hash": identity.git_hash,
        "uptime_secs": start.elapsed().as_secs(),
        "session_count": live_session_ids.len(),
        "spawned_swarm_agent_count": spawned_swarm_agent_count,
        "swarm_member_count": members.len(),
    })
    .to_string()
}

async fn clients_map(
    client_connections: &Arc<RwLock<HashMap<String, ClientConnectionInfo>>>,
    swarm_members: &Arc<RwLock<HashMap<String, SwarmMember>>>,
) -> String {
    let connections = client_connections.read().await;
    let members = swarm_members.read().await;
    let clients: Vec<serde_json::Value> = connections
        .values()
        .map(|info| {
            let member = members.get(&info.session_id);
            serde_json::json!({
                "client_id": info.client_id,
                "session_id": info.session_id,
                "friendly_name": member.and_then(|m| m.friendly_name.clone()),
                "working_dir": member.and_then(|m| m.working_dir.clone()),
                "swarm_id": member.and_then(|m| m.swarm_id.clone()),
                "status": member.map(|m| m.status.clone()),
                "detail": member.and_then(|m| m.detail.clone()),
                "connected_secs_ago": info.connected_at.elapsed().as_secs(),
                "last_seen_secs_ago": info.last_seen.elapsed().as_secs(),
            })
        })
        .collect();
    serde_json::json!({ "count": clients.len(), "clients": clients }).to_string()
}

#[expect(
    clippy::too_many_arguments,
    reason = "debug client wiring fans out across sessions, swarms, and transport state"
)]
pub(super) async fn handle_debug_client(
    stream: Stream,
    sessions: SessionAgents,
    session_id: Arc<RwLock<String>>,
    provider: Arc<dyn Provider>,
    client_connections: Arc<RwLock<HashMap<String, ClientConnectionInfo>>>,
    swarm_members: Arc<RwLock<HashMap<String, SwarmMember>>>,
    swarms_by_id: Arc<RwLock<HashMap<String, HashSet<String>>>>,
    swarm_plans: Arc<RwLock<HashMap<String, VersionedPlan>>>,
    swarm_coordinators: Arc<RwLock<HashMap<String, String>>>,
    server_identity: ServerIdentity,
    server_start_time: Instant,
    mcp_pool: Option<Arc<crate::mcp::SharedMcpPool>>,
    soft_interrupt_queues: SessionInterruptQueues,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }

        let event = match decode_request(&line) {
            Err(e) => ServerEvent::Error {
                id: 0,
                message: format!("Invalid request: {e}"),
                retry_after_secs: None,
            },
            Ok(Request::Ping { id }) => ServerEvent::Pong {
                id,
                native_ssh_protocol: Some(1),
                capabilities: vec!["session_tools".into()],
            },
            Ok(Request::DebugCommand {
                id,
                command,
                session_id: requested_session,
            }) => {
                let command = command.trim();
                let result: Result<String> = match command {
                    "server:info" => {
                        Ok(server_info(&sessions, &swarm_members, &server_identity, server_start_time).await)
                    }
                    "clients:map" => Ok(clients_map(&client_connections, &swarm_members).await),
                    "state" | "history" => {
                        match resolve_debug_session(&sessions, &session_id, requested_session).await {
                            Ok(agent) => {
                                let agent = agent.lock().await;
                                let value = if command == "state" {
                                    serde_json::json!({
                                        "session_id": agent.session_id(),
                                        "messages": agent.message_count(),
                                        "is_canary": agent.is_canary(),
                                        "provider": agent.provider_name(),
                                        "model": agent.provider_model(),
                                        "upstream_provider": agent.last_upstream_provider(),
                                        "server_name": server_identity.name,
                                        "server_icon": server_identity.icon,
                                        "server_version": server_identity.version,
                                    })
                                } else {
                                    serde_json::to_value(agent.get_history()).unwrap_or_default()
                                };
                                Ok(serde_json::to_string_pretty(&value).unwrap_or_default())
                            }
                            Err(e) => Err(e),
                        }
                    }
                    _ => match command.strip_prefix("create_session") {
                        Some(rest) if rest.is_empty() || rest.starts_with(':') => {
                            create_session(
                                command,
                                &sessions,
                                &session_id,
                                &provider,
                                &swarm_members,
                                &swarms_by_id,
                                &swarm_coordinators,
                                &swarm_plans,
                                &soft_interrupt_queues,
                                mcp_pool.clone(),
                            )
                            .await
                        }
                        _ => Err(anyhow::anyhow!(
                            "unknown debug command '{command}' (server:info, clients:map, create_session[:<dir>], state, history)"
                        )),
                    },
                };
                match result {
                    Ok(output) => ServerEvent::DebugResponse { id, ok: true, output },
                    Err(e) => ServerEvent::DebugResponse { id, ok: false, output: e.to_string() },
                }
            }
            Ok(other) => ServerEvent::Error {
                id: other.id(),
                message: "Debug socket only allows ping and debug_command".to_string(),
                retry_after_secs: None,
            },
        };
        writer.write_all(encode_event(&event).as_bytes()).await?;
    }

    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "session creation needs sessions, swarm state, provider template and queues"
)]
async fn create_session(
    command: &str,
    sessions: &SessionAgents,
    session_id: &Arc<RwLock<String>>,
    provider: &Arc<dyn Provider>,
    swarm_members: &Arc<RwLock<HashMap<String, SwarmMember>>>,
    swarms_by_id: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    swarm_coordinators: &Arc<RwLock<HashMap<String, String>>>,
    swarm_plans: &Arc<RwLock<HashMap<String, VersionedPlan>>>,
    soft_interrupt_queues: &SessionInterruptQueues,
    mcp_pool: Option<Arc<crate::mcp::SharedMcpPool>>,
) -> Result<String> {
    let created = create_headless_session(
        sessions,
        session_id,
        provider,
        command,
        swarm_members,
        swarms_by_id,
        swarm_coordinators,
        swarm_plans,
        soft_interrupt_queues,
        false,
        None,
        None,
        None,
        None,
        mcp_pool,
        None,
        super::headless::HeadlessMemoryScope::IsolatedTest,
    )
    .await?;
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&created)
        && let Some(swarm_id) = value.get("swarm_id").and_then(|value| value.as_str())
    {
        let swarm_state = SwarmState {
            members: Arc::clone(swarm_members),
            swarms_by_id: Arc::clone(swarms_by_id),
            plans: Arc::clone(swarm_plans),
            coordinators: Arc::clone(swarm_coordinators),
        };
        persist_swarm_state_for(swarm_id, &swarm_state).await;
    }
    Ok(created)
}
