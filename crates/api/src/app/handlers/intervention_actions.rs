use super::*;

pub(crate) async fn apply_intervention_action(
    state: &ApiState,
    action: &InterventionActionRequest,
) -> Result<InterventionEffectResponse, (StatusCode, Json<ApiErrorResponse>)> {
    match action {
        InterventionActionRequest::TriggerTick { agent_id, tick_id } => {
            let outcome = state
                .orchestrator
                .run_agent_tick(*agent_id, tick_id.clone())
                .await
                .map_err(|error| {
                    internal_error(
                        "intervention_apply_failed",
                        format!("failed to run intervention tick: {error}"),
                    )
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
                            tracing::error!(agent_id = %result.agent_id, error = %error, "failed to persist episodic memory from intervention tick");
                        }

                        Ok(InterventionEffectResponse::Tick {
                            agent_id: result.agent_id,
                            outcome: "applied".to_owned(),
                            tick_id: Some(result.tick_id),
                            event_id: result.event_id,
                            mood_label: Some(result.mood_label),
                            valence: Some(result.valence),
                            arousal: Some(result.arousal),
                        })
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
                        agent_id: *agent_id,
                        reason: "busy".to_owned(),
                        tick_id: tick_id.clone(),
                    });
                    Ok(InterventionEffectResponse::Tick {
                        agent_id: *agent_id,
                        outcome: "skipped_busy".to_owned(),
                        tick_id: tick_id.clone(),
                        event_id: None,
                        mood_label: None,
                        valence: None,
                        arousal: None,
                    })
                }
                AgentTickOrchestratorOutcome::SkippedDuplicate => {
                    state.event_hub.publish(WsServerEvent::TickSkipped {
                        agent_id: *agent_id,
                        reason: "duplicate".to_owned(),
                        tick_id: tick_id.clone(),
                    });
                    Ok(InterventionEffectResponse::Tick {
                        agent_id: *agent_id,
                        outcome: "skipped_duplicate".to_owned(),
                        tick_id: tick_id.clone(),
                        event_id: None,
                        mood_label: None,
                        valence: None,
                        arousal: None,
                    })
                }
            }
        }
        InterventionActionRequest::AppendMemory {
            agent_id,
            content,
            importance,
        } => {
            if content.trim().is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResponse {
                        error: "invalid_content",
                        message: "memory content must not be empty".to_owned(),
                    }),
                ));
            }

            let agent_exists = state
                .repository
                .get_agent(*agent_id)
                .await
                .map_err(|error| {
                    internal_error(
                        "intervention_apply_failed",
                        format!("failed to load intervention target agent: {error}"),
                    )
                })?
                .is_some();
            if !agent_exists {
                return Err((
                    StatusCode::NOT_FOUND,
                    Json(ApiErrorResponse {
                        error: "agent_not_found",
                        message: format!("agent `{agent_id}` does not exist"),
                    }),
                ));
            }

            let memory = state
                .memory_service
                .append_memory(*agent_id, content.clone(), importance.unwrap_or(0.6))
                .await
                .map_err(|error| {
                    internal_error(
                        "intervention_apply_failed",
                        format!("failed to append memory intervention: {error}"),
                    )
                })?;

            Ok(InterventionEffectResponse::Memory {
                agent_id: *agent_id,
                memory_id: memory.id,
                embedding_status: memory.embedding_status,
            })
        }
        InterventionActionRequest::SendMessage {
            sender_agent_id,
            receiver_agent_id,
            content,
        } => {
            let message = enqueue_agent_message(
                state.repository.as_ref(),
                *sender_agent_id,
                *receiver_agent_id,
                content.clone(),
            )
            .await?;
            Ok(InterventionEffectResponse::Message {
                message_id: message.id,
                status: message.status,
            })
        }
        InterventionActionRequest::AppendEvent {
            agent_id,
            event_type,
            description,
            payload_json,
        } => {
            if event_type.trim().is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResponse {
                        error: "invalid_event_type",
                        message: "event_type must not be empty".to_owned(),
                    }),
                ));
            }
            if description.trim().is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResponse {
                        error: "invalid_description",
                        message: "description must not be empty".to_owned(),
                    }),
                ));
            }
            if let Some(agent_id) = agent_id {
                let exists = state
                    .repository
                    .get_agent(*agent_id)
                    .await
                    .map_err(|error| {
                        internal_error(
                            "intervention_apply_failed",
                            format!("failed to load event target agent: {error}"),
                        )
                    })?
                    .is_some();
                if !exists {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(ApiErrorResponse {
                            error: "agent_not_found",
                            message: format!("agent `{agent_id}` does not exist"),
                        }),
                    ));
                }
            }

            let event = state
                .repository
                .append_agent_event(&NewAgentEvent {
                    agent_id: *agent_id,
                    event_type: event_type.trim().to_owned(),
                    description: description.trim().to_owned(),
                    payload_json: payload_json.clone().unwrap_or_else(|| json!({})),
                })
                .await
                .map_err(|error| {
                    internal_error(
                        "intervention_apply_failed",
                        format!("failed to append intervention event: {error}"),
                    )
                })?;

            Ok(InterventionEffectResponse::Event {
                event_id: event.id,
                event_type: event.event_type,
            })
        }
        InterventionActionRequest::SetTimeScale { time_scale } => {
            let time_scale = validate_time_scale(*time_scale)?;
            let updated = state
                .repository
                .set_time_scale(time_scale)
                .await
                .map_err(|error| {
                    internal_error(
                        "intervention_apply_failed",
                        format!("failed to update simulation time scale: {error}"),
                    )
                })?;

            Ok(InterventionEffectResponse::TimeScale {
                time_scale: updated.time_scale,
                updated_at: updated.updated_at.to_rfc3339(),
            })
        }
    }
}

pub(crate) async fn enqueue_agent_message(
    repository: &dyn AgentCoreRepository,
    sender_agent_id: Uuid,
    receiver_agent_id: Uuid,
    content: String,
) -> Result<MessageRecord, (StatusCode, Json<ApiErrorResponse>)> {
    if content.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_content",
                message: "message content must not be empty".to_owned(),
            }),
        ));
    }
    if sender_agent_id == receiver_agent_id {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_sender",
                message: "sender and receiver must be different agents".to_owned(),
            }),
        ));
    }

    let sender_exists = repository
        .get_agent(sender_agent_id)
        .await
        .map_err(|error| {
            internal_error(
                "message_send_failed",
                format!("failed to load sender agent: {error}"),
            )
        })?
        .is_some();
    if !sender_exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse {
                error: "sender_not_found",
                message: format!("sender agent `{sender_agent_id}` does not exist"),
            }),
        ));
    }

    let receiver_exists = repository
        .get_agent(receiver_agent_id)
        .await
        .map_err(|error| {
            internal_error(
                "message_send_failed",
                format!("failed to load receiver agent: {error}"),
            )
        })?
        .is_some();
    if !receiver_exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse {
                error: "receiver_not_found",
                message: format!("receiver agent `{receiver_agent_id}` does not exist"),
            }),
        ));
    }

    repository
        .enqueue_message(&NewMessage {
            sender_type: "agent".to_owned(),
            sender_id: Some(sender_agent_id),
            receiver_agent_id,
            content: content.trim().to_owned(),
        })
        .await
        .map_err(|error| {
            internal_error(
                "message_send_failed",
                format!("failed to enqueue agent message: {error}"),
            )
        })
}
