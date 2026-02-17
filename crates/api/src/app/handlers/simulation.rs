use super::*;

pub(crate) async fn trigger_agent_tick(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
    payload: Option<Json<TriggerTickRequest>>,
) -> Result<(StatusCode, Json<TriggerTickResponse>), (StatusCode, Json<ApiErrorResponse>)> {
    let tick_id = payload.and_then(|Json(body)| body.tick_id);
    let outcome = state
        .orchestrator
        .run_agent_tick(agent_id, tick_id.clone())
        .await
        .map_err(|error| {
            internal_error("tick_failed", format!("failed to run agent tick: {error}"))
        })?;

    match outcome {
        AgentTickOrchestratorOutcome::Executed(result) => match result.status {
            AgentTickExecutionStatus::Applied => {
                if let Err(error) = state
                    .memory_service
                    .append_memory(
                        result.agent_id,
                        format!(
                            "{} (mood={}, valence={:.2}, arousal={:.2})",
                            result.action_summary,
                            result.mood_label,
                            result.valence,
                            result.arousal
                        ),
                        0.7,
                    )
                    .await
                {
                    tracing::error!(agent_id = %result.agent_id, error = %error, "failed to persist episodic memory from tick");
                }

                Ok((
                    StatusCode::OK,
                    Json(TriggerTickResponse {
                        outcome: "applied",
                        agent_id: result.agent_id,
                        tick_id: Some(result.tick_id),
                        event_id: result.event_id,
                        mood_label: Some(result.mood_label),
                        valence: Some(result.valence),
                        arousal: Some(result.arousal),
                    }),
                ))
            }
            AgentTickExecutionStatus::AgentMissing => Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResponse {
                    error: "agent_not_found",
                    message: format!("agent `{}` does not exist", result.agent_id),
                }),
            )),
        },
        AgentTickOrchestratorOutcome::SkippedBusy => {
            state.event_hub.publish(WsServerEvent::TickSkipped {
                agent_id,
                reason: "busy".to_owned(),
                tick_id: tick_id.clone(),
            });
            Ok((
                StatusCode::CONFLICT,
                Json(TriggerTickResponse {
                    outcome: "skipped_busy",
                    agent_id,
                    tick_id,
                    event_id: None,
                    mood_label: None,
                    valence: None,
                    arousal: None,
                }),
            ))
        }
        AgentTickOrchestratorOutcome::SkippedDuplicate => {
            state.event_hub.publish(WsServerEvent::TickSkipped {
                agent_id,
                reason: "duplicate".to_owned(),
                tick_id: tick_id.clone(),
            });
            Ok((
                StatusCode::CONFLICT,
                Json(TriggerTickResponse {
                    outcome: "skipped_duplicate",
                    agent_id,
                    tick_id,
                    event_id: None,
                    mood_label: None,
                    valence: None,
                    arousal: None,
                }),
            ))
        }
    }
}

pub(crate) async fn get_agent_state(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
) -> Result<Json<AgentStateResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let Some(record) = state
        .repository
        .get_agent_state(agent_id)
        .await
        .map_err(|error| {
            internal_error(
                "state_read_failed",
                format!("failed to read agent state: {error}"),
            )
        })?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse {
                error: "state_not_found",
                message: format!("state for agent `{agent_id}` was not found"),
            }),
        ));
    };

    Ok(Json(AgentStateResponse {
        agent_id: record.agent_id,
        mood_label: record.mood_label,
        valence: record.valence,
        arousal: record.arousal,
        updated_at: record.updated_at.to_rfc3339(),
    }))
}

pub(crate) async fn get_agent_inspector(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
    Query(query): Query<AgentInspectorQuery>,
) -> Result<Json<AgentInspectorResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let events_limit = query
        .events_limit
        .unwrap_or(DEFAULT_INSPECTOR_LIMIT)
        .clamp(1, 200);
    let messages_limit = query
        .messages_limit
        .unwrap_or(DEFAULT_INSPECTOR_LIMIT)
        .clamp(1, 200);
    let relationships_limit = query
        .relationships_limit
        .unwrap_or(DEFAULT_INSPECTOR_LIMIT)
        .clamp(1, 200);
    let memories_limit = query
        .memories_limit
        .unwrap_or(DEFAULT_INSPECTOR_LIMIT)
        .clamp(1, 200);
    let timeline_limit = query
        .timeline_limit
        .unwrap_or(DEFAULT_INSPECTOR_LIMIT)
        .clamp(1, 200);
    let recall_query = query
        .recall_query
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let recall_top_k = query
        .recall_top_k
        .unwrap_or(DEFAULT_RECALL_TOP_K)
        .clamp(1, 50);

    let repository = state.repository.clone();
    let memory_service = state.memory_service.clone();

    let (
        agent_result,
        state_result,
        events_result,
        messages_result,
        relationships_result,
        timeline_messages_result,
        memories_result,
    ) = tokio::join!(
        repository.get_agent(agent_id),
        repository.get_agent_state(agent_id),
        repository.list_agent_events(Some(agent_id), events_limit),
        repository.list_agent_messages(agent_id, messages_limit),
        repository.list_agent_relationships(agent_id, relationships_limit),
        repository.list_agent_message_timeline(agent_id, timeline_limit),
        memory_service.list_recent_memories(agent_id, memories_limit),
    );

    let Some(agent) = agent_result.map_err(|error| {
        internal_error(
            "inspector_read_failed",
            format!("failed to read agent profile: {error}"),
        )
    })?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse {
                error: "agent_not_found",
                message: format!("agent `{agent_id}` does not exist"),
            }),
        ));
    };

    let state_item = state_result
        .map_err(|error| {
            internal_error(
                "inspector_read_failed",
                format!("failed to read agent state: {error}"),
            )
        })?
        .map(|record| AgentStateResponse {
            agent_id: record.agent_id,
            mood_label: record.mood_label,
            valence: record.valence,
            arousal: record.arousal,
            updated_at: record.updated_at.to_rfc3339(),
        });

    let events = events_result.map_err(|error| {
        internal_error(
            "inspector_read_failed",
            format!("failed to read agent events: {error}"),
        )
    })?;
    let messages = messages_result.map_err(|error| {
        internal_error(
            "inspector_read_failed",
            format!("failed to read agent messages: {error}"),
        )
    })?;
    let relationships = relationships_result.map_err(|error| {
        internal_error(
            "inspector_read_failed",
            format!("failed to read agent relationships: {error}"),
        )
    })?;
    let memories = memories_result.map_err(|error| {
        internal_error(
            "inspector_read_failed",
            format!("failed to read agent memories: {error}"),
        )
    })?;
    let timeline_messages = timeline_messages_result.map_err(|error| {
        internal_error(
            "inspector_read_failed",
            format!("failed to read agent relationship timeline: {error}"),
        )
    })?;

    let recall = if let Some(recall_query) = recall_query {
        let recalled = state
            .memory_service
            .recall(agent_id, &recall_query, recall_top_k)
            .await
            .map_err(|error| {
                internal_error(
                    "inspector_recall_failed",
                    format!("failed to run inspector memory recall: {error}"),
                )
            })?;
        Some(AgentInspectorRecallResponse {
            query: recall_query,
            top_k: recall_top_k,
            items: recalled.into_iter().map(map_recall_item).collect(),
        })
    } else {
        None
    };

    let recent_events: Vec<EventItemResponse> = events.iter().map(map_event_record).collect();
    let recent_messages: Vec<MessageItemResponse> =
        messages.into_iter().map(map_message_record).collect();
    let recent_relationships: Vec<RelationshipItemResponse> = relationships
        .into_iter()
        .map(map_relationship_record)
        .collect();
    let relationship_timeline =
        build_relationship_timeline(&state, agent_id, timeline_messages, &recent_relationships)
            .await?;
    let recent_memories: Vec<InspectorMemoryItemResponse> =
        memories.into_iter().map(map_inspector_memory).collect();

    Ok(Json(AgentInspectorResponse {
        agent: AgentInspectorAgentResponse {
            id: agent.id,
            name: agent.name,
            avatar_url: agent.avatar_url,
            personality_json: agent.personality_json,
            created_at: agent.created_at.to_rfc3339(),
        },
        state: state_item,
        summary: AgentInspectorSummaryResponse {
            events_count: recent_events.len(),
            messages_count: recent_messages.len(),
            relationships_count: recent_relationships.len(),
            timeline_count: relationship_timeline.len(),
            memories_count: recent_memories.len(),
        },
        recent_events,
        recent_messages,
        recent_relationships,
        relationship_timeline,
        recent_memories,
        recall,
    }))
}
