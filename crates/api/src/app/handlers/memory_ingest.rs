use super::*;

pub(crate) async fn append_agent_memory(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
    Json(payload): Json<AppendMemoryRequest>,
) -> Result<(StatusCode, Json<AppendMemoryResponse>), (StatusCode, Json<ApiErrorResponse>)> {
    if payload.content.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_content",
                message: "memory content must not be empty".to_owned(),
            }),
        ));
    }

    let memory = state
        .memory_service
        .append_memory(agent_id, payload.content, payload.importance.unwrap_or(0.6))
        .await
        .map_err(|error| {
            internal_error(
                "memory_append_failed",
                format!("failed to append memory: {error}"),
            )
        })?;

    Ok((
        StatusCode::CREATED,
        Json(AppendMemoryResponse {
            memory_id: memory.id,
            embedding_status: memory.embedding_status,
        }),
    ))
}
