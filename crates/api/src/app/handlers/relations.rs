use super::*;

pub(crate) async fn build_relationship_timeline(
    state: &ApiState,
    agent_id: Uuid,
    messages: Vec<MessageRecord>,
    relationships: &[RelationshipItemResponse],
) -> Result<Vec<RelationshipTimelineItemResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let mut relationship_by_counterpart: HashMap<Uuid, RelationshipItemResponse> = HashMap::new();
    for edge in relationships {
        if let Some(counterpart_id) = relationship_counterpart_agent_id(edge, agent_id) {
            relationship_by_counterpart.insert(counterpart_id, edge.clone());
        }
    }

    let mut counterpart_ids = HashSet::new();
    for message in &messages {
        if let Some(counterpart_id) = timeline_counterpart_agent_id(message, agent_id) {
            counterpart_ids.insert(counterpart_id);
        }
    }

    let mut counterparts: HashMap<Uuid, (String, Option<String>)> = HashMap::new();
    for counterpart_id in counterpart_ids {
        let record = state
            .repository
            .get_agent(counterpart_id)
            .await
            .map_err(|error| {
                internal_error(
                    "relationship_history_failed",
                    format!("failed to load counterpart agent profile: {error}"),
                )
            })?;
        let (name, avatar_url) = match record {
            Some(agent) => (agent.name, agent.avatar_url),
            None => (format!("agent-{counterpart_id}"), None),
        };
        counterparts.insert(counterpart_id, (name, avatar_url));
    }

    let mut items = Vec::with_capacity(messages.len());
    for message in messages {
        let direction = if message.sender_id == Some(agent_id) {
            "outgoing"
        } else {
            "incoming"
        };
        let counterpart_agent_id = timeline_counterpart_agent_id(&message, agent_id);
        let (counterpart_name, counterpart_avatar_url) = counterpart_agent_id
            .and_then(|id| counterparts.get(&id).cloned())
            .map(|(name, avatar_url)| (Some(name), avatar_url))
            .unwrap_or((None, None));
        let relationship =
            counterpart_agent_id.and_then(|id| relationship_by_counterpart.get(&id).cloned());

        items.push(RelationshipTimelineItemResponse {
            message_id: message.id,
            direction: direction.to_owned(),
            counterpart_agent_id,
            counterpart_name,
            counterpart_avatar_url,
            content: message.content,
            status: message.status,
            created_at: message.created_at.to_rfc3339(),
            relationship,
        });
    }

    Ok(items)
}

pub(crate) fn timeline_counterpart_agent_id(
    message: &MessageRecord,
    agent_id: Uuid,
) -> Option<Uuid> {
    if message.sender_id == Some(agent_id) {
        return Some(message.receiver_agent_id);
    }
    if message.receiver_agent_id == agent_id {
        return message.sender_id;
    }
    message.sender_id
}

pub(crate) fn relationship_counterpart_agent_id(
    edge: &RelationshipItemResponse,
    agent_id: Uuid,
) -> Option<Uuid> {
    if edge.agent_a == agent_id {
        return Some(edge.agent_b);
    }
    if edge.agent_b == agent_id {
        return Some(edge.agent_a);
    }
    None
}

pub(crate) fn map_event_record(record: &AgentEventRecord) -> EventItemResponse {
    EventItemResponse {
        id: record.id,
        agent_id: record.agent_id,
        event_type: record.event_type.clone(),
        description: record.description.clone(),
        payload: record.payload_json.to_string(),
        occurred_at: record.occurred_at.to_rfc3339(),
    }
}

pub(crate) fn map_relationship_update_event(
    record: &AgentEventRecord,
) -> Option<RelationshipItemResponse> {
    if record.event_type != "agent.relationship.updated" {
        return None;
    }

    let payload = record.payload_json.as_object()?;
    let id = payload.get("relationship_id")?.as_i64()?;
    let agent_a = Uuid::parse_str(payload.get("agent_a")?.as_str()?).ok()?;
    let agent_b = Uuid::parse_str(payload.get("agent_b")?.as_str()?).ok()?;
    let affinity_score = payload.get("affinity_score")?.as_f64()? as f32;
    let history_summary = payload
        .get("history_summary")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_owned();
    let last_interaction_at = payload
        .get("last_interaction_at")
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    let created_at = payload
        .get("created_at")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| record.occurred_at.to_rfc3339());

    Some(RelationshipItemResponse {
        id,
        agent_a,
        agent_b,
        affinity_score,
        history_summary,
        last_interaction_at,
        created_at,
    })
}

pub(crate) async fn build_relationship_graph(
    state: &ApiState,
    agent_id: Option<Uuid>,
    limit_edges: u32,
) -> anyhow::Result<RelationshipGraphResponse> {
    let edges_raw = match agent_id {
        Some(agent_id) => {
            state
                .repository
                .list_agent_relationships(agent_id, limit_edges)
                .await?
        }
        None => state.repository.list_relationships(limit_edges).await?,
    };

    let edges: Vec<RelationshipItemResponse> = edges_raw
        .iter()
        .cloned()
        .map(map_relationship_record)
        .collect();

    let mut participant_ids = HashSet::new();
    for edge in &edges {
        participant_ids.insert(edge.agent_a);
        participant_ids.insert(edge.agent_b);
    }

    if let Some(agent_id) = agent_id {
        participant_ids.insert(agent_id);
    }

    if participant_ids.is_empty() {
        let agents = state.repository.list_agents(limit_edges).await?;
        for agent in agents {
            participant_ids.insert(agent.id);
        }
    }

    let mut nodes = Vec::with_capacity(participant_ids.len());
    for participant_id in participant_ids {
        let agent = state.repository.get_agent(participant_id).await?;
        let (name, avatar_url) = match agent {
            Some(agent) => (agent.name, agent.avatar_url),
            None => (format!("agent-{participant_id}"), None),
        };
        nodes.push(RelationshipGraphNodeResponse {
            agent_id: participant_id,
            name,
            avatar_url,
        });
    }
    nodes.sort_by(|left, right| left.name.cmp(&right.name));

    Ok(RelationshipGraphResponse { nodes, edges })
}
