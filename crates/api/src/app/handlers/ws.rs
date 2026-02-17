use super::*;

pub(crate) async fn ws_events(
    State(state): State<ApiState>,
    Query(query): Query<WsEventsQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_events_session(socket, state, query))
}

pub(crate) async fn ws_relationships(
    State(state): State<ApiState>,
    Query(query): Query<WsRelationshipsQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_relationships_session(socket, state, query))
}

pub(crate) async fn ws_events_session(
    mut socket: WebSocket,
    state: ApiState,
    query: WsEventsQuery,
) {
    let snapshot_limit = query
        .snapshot_limit
        .unwrap_or(DEFAULT_WS_SNAPSHOT_LIMIT)
        .clamp(1, 200);
    let snapshot = state
        .repository
        .list_agent_events(query.agent_id, snapshot_limit)
        .await;

    match snapshot {
        Ok(records) => {
            let items = records.iter().map(map_event_record).collect();
            let event = WsServerEvent::Snapshot { items };
            if !send_ws_event(&mut socket, &event).await {
                return;
            }
        }
        Err(error) => {
            let event = WsServerEvent::Error {
                message: format!("failed to load snapshot: {error}"),
            };
            let _ = send_ws_event(&mut socket, &event).await;
            return;
        }
    }

    let mut receiver = state.event_hub.subscribe();
    loop {
        match receiver.recv().await {
            Ok(event) => {
                if matches!(event, WsServerEvent::RelationshipUpdated { .. }) {
                    continue;
                }
                if !ws_event_matches_agent(&event, query.agent_id) {
                    continue;
                }
                if !send_ws_event(&mut socket, &event).await {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(
                    skipped_messages = skipped,
                    "disconnecting lagging websocket client"
                );
                let _ = send_ws_event(
                    &mut socket,
                    &WsServerEvent::Error {
                        message: format!(
                            "stream lagged by {skipped} events; reconnect for fresh state"
                        ),
                    },
                )
                .await;
                break;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

pub(crate) async fn ws_relationships_session(
    mut socket: WebSocket,
    state: ApiState,
    query: WsRelationshipsQuery,
) {
    let snapshot_limit = query
        .snapshot_limit
        .unwrap_or(DEFAULT_RELATIONSHIP_GRAPH_LIMIT)
        .clamp(1, 500);

    let snapshot = build_relationship_graph(&state, query.agent_id, snapshot_limit).await;
    match snapshot {
        Ok(graph) => {
            if !send_ws_relationship_event(
                &mut socket,
                &WsRelationshipServerEvent::Snapshot { graph },
            )
            .await
            {
                return;
            }
        }
        Err(error) => {
            let _ = send_ws_relationship_event(
                &mut socket,
                &WsRelationshipServerEvent::Error {
                    message: format!("failed to load relationship graph snapshot: {error}"),
                },
            )
            .await;
            return;
        }
    }

    let mut receiver = state.event_hub.subscribe();
    loop {
        match receiver.recv().await {
            Ok(WsServerEvent::RelationshipUpdated { edge }) => {
                if !relationship_edge_matches_agent(&edge, query.agent_id) {
                    continue;
                }
                if !send_ws_relationship_event(
                    &mut socket,
                    &WsRelationshipServerEvent::EdgeUpdated { edge },
                )
                .await
                {
                    break;
                }
            }
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(
                    skipped_messages = skipped,
                    "disconnecting lagging relationship websocket client"
                );
                let _ = send_ws_relationship_event(
                    &mut socket,
                    &WsRelationshipServerEvent::Error {
                        message: format!(
                            "relationship stream lagged by {skipped} events; reconnect for fresh state"
                        ),
                    },
                )
                .await;
                break;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

pub(crate) async fn send_ws_event(socket: &mut WebSocket, event: &WsServerEvent) -> bool {
    let payload = match serde_json::to_string(event) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::error!(error = %error, "failed to serialize websocket event");
            return false;
        }
    };

    match socket.send(Message::Text(payload.into())).await {
        Ok(_) => true,
        Err(error) => {
            tracing::debug!(error = %error, "websocket send failed");
            false
        }
    }
}

pub(crate) async fn send_ws_relationship_event(
    socket: &mut WebSocket,
    event: &WsRelationshipServerEvent,
) -> bool {
    let payload = match serde_json::to_string(event) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::error!(error = %error, "failed to serialize relationship websocket event");
            return false;
        }
    };

    match socket.send(Message::Text(payload.into())).await {
        Ok(_) => true,
        Err(error) => {
            tracing::debug!(error = %error, "relationship websocket send failed");
            false
        }
    }
}

pub(crate) fn ws_event_matches_agent(event: &WsServerEvent, agent_id: Option<Uuid>) -> bool {
    let Some(agent_id) = agent_id else {
        return true;
    };

    match event {
        WsServerEvent::Snapshot { .. } | WsServerEvent::Error { .. } => true,
        WsServerEvent::TickSkipped {
            agent_id: event_agent_id,
            ..
        } => *event_agent_id == agent_id,
        WsServerEvent::EventAppended { item } => item.agent_id == Some(agent_id),
        WsServerEvent::RelationshipUpdated { edge } => {
            edge.agent_a == agent_id || edge.agent_b == agent_id
        }
    }
}

pub(crate) fn relationship_edge_matches_agent(
    edge: &RelationshipItemResponse,
    agent_id: Option<Uuid>,
) -> bool {
    match agent_id {
        Some(agent_id) => edge.agent_a == agent_id || edge.agent_b == agent_id,
        None => true,
    }
}
