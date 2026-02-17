use super::*;

pub(crate) async fn list_agent_relationships(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
    Query(query): Query<RelationshipListQuery>,
) -> Result<Json<AgentRelationshipsResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let items = state
        .repository
        .list_agent_relationships(agent_id, limit)
        .await
        .map_err(|error| {
            internal_error(
                "relationship_list_failed",
                format!("failed to list agent relationships: {error}"),
            )
        })?
        .into_iter()
        .map(map_relationship_record)
        .collect();

    Ok(Json(AgentRelationshipsResponse { items }))
}

pub(crate) async fn list_agent_relationship_history(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
    Query(query): Query<RelationshipTimelineQuery>,
) -> Result<Json<AgentRelationshipTimelineResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let messages = state
        .repository
        .list_agent_message_timeline(agent_id, limit)
        .await
        .map_err(|error| {
            internal_error(
                "relationship_history_failed",
                format!("failed to read relationship message timeline: {error}"),
            )
        })?;
    let relationships = state
        .repository
        .list_agent_relationships(agent_id, limit.clamp(1, 200))
        .await
        .map_err(|error| {
            internal_error(
                "relationship_history_failed",
                format!("failed to read relationship snapshots: {error}"),
            )
        })?;
    let relationship_items: Vec<RelationshipItemResponse> = relationships
        .into_iter()
        .map(map_relationship_record)
        .collect();

    let items =
        build_relationship_timeline(&state, agent_id, messages, &relationship_items).await?;
    Ok(Json(AgentRelationshipTimelineResponse { agent_id, items }))
}

pub(crate) async fn get_relationship_graph(
    State(state): State<ApiState>,
    Query(query): Query<RelationshipGraphQuery>,
) -> Result<Json<RelationshipGraphResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit_edges = query
        .limit_edges
        .unwrap_or(DEFAULT_RELATIONSHIP_GRAPH_LIMIT)
        .clamp(1, 500);

    let graph = build_relationship_graph(&state, query.agent_id, limit_edges)
        .await
        .map_err(|error| {
            internal_error(
                "relationship_graph_failed",
                format!("failed to build relationship graph: {error}"),
            )
        })?;

    Ok(Json(graph))
}
