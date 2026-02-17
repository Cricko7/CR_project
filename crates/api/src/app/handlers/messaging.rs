use super::*;

pub(crate) async fn send_agent_message(
    State(state): State<ApiState>,
    Path(receiver_agent_id): Path<Uuid>,
    Json(payload): Json<CreateMessageRequest>,
) -> Result<(StatusCode, Json<CreateMessageResponse>), (StatusCode, Json<ApiErrorResponse>)> {
    let message = enqueue_agent_message(
        state.repository.as_ref(),
        payload.sender_agent_id,
        receiver_agent_id,
        payload.content,
    )
    .await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(CreateMessageResponse {
            message_id: message.id,
            status: message.status,
        }),
    ))
}

pub(crate) async fn list_agent_messages(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
    Query(query): Query<MessageListQuery>,
) -> Result<Json<AgentMessagesResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let items = state
        .repository
        .list_agent_messages(agent_id, limit)
        .await
        .map_err(|error| {
            internal_error(
                "message_list_failed",
                format!("failed to list agent messages: {error}"),
            )
        })?
        .into_iter()
        .map(map_message_record)
        .collect();

    Ok(Json(AgentMessagesResponse { items }))
}
