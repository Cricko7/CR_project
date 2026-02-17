use super::*;

pub(crate) async fn create_intervention(
    State(state): State<ApiState>,
    Json(payload): Json<CreateInterventionRequest>,
) -> Result<(StatusCode, Json<CreateInterventionResponse>), (StatusCode, Json<ApiErrorResponse>)> {
    let admin_user_id = payload.admin_user_id.trim().to_owned();
    if admin_user_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_admin_user_id",
                message: "admin_user_id must not be empty".to_owned(),
            }),
        ));
    }

    let action_type = payload.action.action_type().to_owned();
    let action_payload = payload.action.payload_json();

    let effect = match apply_intervention_action(&state, &payload.action).await {
        Ok(effect) => effect,
        Err((status, Json(error_body))) => {
            let failed_payload = json!({
                "action": action_payload,
                "error": {
                    "code": error_body.error,
                    "message": error_body.message,
                }
            });
            if let Err(store_error) = state
                .repository
                .append_intervention(&NewIntervention {
                    admin_user_id,
                    action_type,
                    payload_json: failed_payload,
                    result_status: "failed".to_owned(),
                })
                .await
            {
                tracing::error!(error = %store_error, "failed to persist failed intervention record");
            }
            return Err((status, Json(error_body)));
        }
    };

    let record = state
        .repository
        .append_intervention(&NewIntervention {
            admin_user_id,
            action_type,
            payload_json: json!({
                "action": action_payload,
                "effect": effect.clone(),
            }),
            result_status: "applied".to_owned(),
        })
        .await
        .map_err(|error| {
            internal_error(
                "intervention_store_failed",
                format!("failed to store intervention: {error}"),
            )
        })?;

    Ok((
        StatusCode::OK,
        Json(CreateInterventionResponse {
            intervention: map_intervention_record(record),
            effect,
        }),
    ))
}

pub(crate) async fn get_simulation_time_scale(
    State(state): State<ApiState>,
) -> Result<Json<SimulationTimeScaleResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let record = state.repository.get_time_scale().await.map_err(|error| {
        internal_error(
            "simulation_time_scale_read_failed",
            format!("failed to read simulation time scale: {error}"),
        )
    })?;

    Ok(Json(map_simulation_time_scale_record(record)))
}

pub(crate) async fn set_simulation_time_scale(
    State(state): State<ApiState>,
    Json(payload): Json<SetSimulationTimeScaleRequest>,
) -> Result<Json<SimulationTimeScaleResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let time_scale = validate_time_scale(payload.time_scale)?;
    let previous_time_scale = state
        .repository
        .get_time_scale()
        .await
        .ok()
        .map(|record| record.time_scale);

    let record = state
        .repository
        .set_time_scale(time_scale)
        .await
        .map_err(|error| {
            internal_error(
                "simulation_time_scale_update_failed",
                format!("failed to update simulation time scale: {error}"),
            )
        })?;

    if let Err(error) = state
        .repository
        .append_agent_event(&NewAgentEvent {
            agent_id: None,
            event_type: "simulation.time_scale.updated".to_owned(),
            description: format!("Simulation time scale updated to {:.2}", record.time_scale),
            payload_json: json!({
                "time_scale": record.time_scale,
                "previous_time_scale": previous_time_scale,
                "updated_at": record.updated_at.to_rfc3339(),
            }),
        })
        .await
    {
        tracing::warn!(error = %error, "failed to append simulation time-scale update event");
    }

    Ok(Json(map_simulation_time_scale_record(record)))
}

pub(crate) async fn list_interventions(
    State(state): State<ApiState>,
    Query(query): Query<InterventionListQuery>,
) -> Result<Json<InterventionsResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit = query
        .limit
        .unwrap_or(DEFAULT_INTERVENTION_LIMIT)
        .clamp(1, 200);
    let items = state
        .repository
        .list_interventions(limit)
        .await
        .map_err(|error| {
            internal_error(
                "intervention_list_failed",
                format!("failed to list interventions: {error}"),
            )
        })?
        .into_iter()
        .map(map_intervention_record)
        .collect();

    Ok(Json(InterventionsResponse { items }))
}

pub(crate) async fn list_events(
    State(state): State<ApiState>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<EventsResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let records = if let Some(after_id) = query.after_id {
        state
            .repository
            .list_agent_events_after_id(query.agent_id, after_id, limit)
            .await
            .map_err(|error| {
                internal_error(
                    "events_read_failed",
                    format!("failed to read events after id: {error}"),
                )
            })?
    } else {
        state
            .repository
            .list_agent_events(query.agent_id, limit)
            .await
            .map_err(|error| {
                internal_error(
                    "events_read_failed",
                    format!("failed to read events: {error}"),
                )
            })?
    };

    let next_after_id = if let Some(after_id) = query.after_id {
        Some(
            records
                .iter()
                .map(|record| record.id)
                .max()
                .unwrap_or(after_id),
        )
    } else {
        None
    };

    let items = records.iter().map(map_event_record).collect();

    Ok(Json(EventsResponse {
        items,
        next_after_id,
    }))
}
