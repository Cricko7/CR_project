use super::*;

pub(crate) async fn recall_agent_memory(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
    Query(query): Query<RecallQuery>,
) -> Result<Json<RecallResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    if query.query.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_query",
                message: "recall query must not be empty".to_owned(),
            }),
        ));
    }

    let top_k = query.top_k.unwrap_or(DEFAULT_RECALL_TOP_K).clamp(1, 50);
    let recalled = state
        .memory_service
        .recall(agent_id, &query.query, top_k)
        .await
        .map_err(|error| {
            internal_error(
                "memory_recall_failed",
                format!("failed to recall memory: {error}"),
            )
        })?;

    let items = recalled.into_iter().map(map_recall_item).collect();
    Ok(Json(RecallResponse { items }))
}

pub(crate) async fn summarize_agent_memory(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
    payload: Option<Json<SummarizeMemoryRequest>>,
) -> Result<Json<SummarizeMemoryResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let body = payload.map(|Json(value)| value);
    let max_active = body
        .as_ref()
        .and_then(|v| v.max_active)
        .unwrap_or(state.memory_defaults.summary_max_active);
    let batch_size = body
        .as_ref()
        .and_then(|v| v.batch_size)
        .unwrap_or(state.memory_defaults.summary_batch_size);

    let result = state
        .memory_service
        .summarize_overflow(agent_id, max_active, batch_size)
        .await
        .map_err(|error| {
            internal_error(
                "memory_summarize_failed",
                format!("failed to summarize memory overflow: {error}"),
            )
        })?;

    Ok(Json(SummarizeMemoryResponse {
        created_summary: result.created_summary,
        source_count: result.source_count,
        summary_entry_id: result.summary_entry_id,
    }))
}

pub(crate) async fn process_memory_embeddings(
    State(state): State<ApiState>,
    payload: Option<Json<ProcessEmbeddingsRequest>>,
) -> Result<Json<ProcessEmbeddingsResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit = payload
        .map(|Json(value)| value.limit.unwrap_or(50))
        .unwrap_or(50)
        .clamp(1, 200);
    let summary = state
        .memory_service
        .process_pending_embeddings(limit)
        .await
        .map_err(|error| {
            internal_error(
                "memory_embedding_failed",
                format!("failed to process memory embeddings: {error}"),
            )
        })?;

    Ok(Json(ProcessEmbeddingsResponse {
        processed: summary.processed,
        succeeded: summary.succeeded,
        failed: summary.failed,
        retried: summary.retried,
        dead_lettered: summary.dead_lettered,
    }))
}

pub(crate) async fn list_dead_letter_embeddings(
    State(state): State<ApiState>,
    Query(query): Query<DeadLetterQuery>,
) -> Result<Json<DeadLetterEmbeddingsResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let entries = state
        .memory_service
        .list_dead_letter_embeddings(limit)
        .await
        .map_err(|error| {
            internal_error(
                "memory_dead_letter_read_failed",
                format!("failed to list dead-letter memory embeddings: {error}"),
            )
        })?;

    Ok(Json(DeadLetterEmbeddingsResponse {
        items: entries.into_iter().map(map_dead_letter_memory).collect(),
    }))
}

pub(crate) async fn requeue_dead_letter_embedding(
    State(state): State<ApiState>,
    Path(memory_id): Path<i64>,
) -> Result<Json<RequeueDeadLetterResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let requeued = state
        .memory_service
        .requeue_dead_letter_embedding(memory_id)
        .await
        .map_err(|error| {
            internal_error(
                "memory_dead_letter_requeue_failed",
                format!("failed to requeue dead-letter memory embedding: {error}"),
            )
        })?;

    Ok(Json(RequeueDeadLetterResponse {
        memory_id,
        requeued,
    }))
}
