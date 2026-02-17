use super::*;

#[derive(Serialize)]
pub(super) struct HealthResponse {
    pub(super) status: &'static str,
    pub(super) service: String,
}

#[derive(Deserialize)]
pub(super) struct TriggerTickRequest {
    pub(super) tick_id: Option<String>,
}

#[derive(Serialize)]
pub(super) struct TriggerTickResponse {
    pub(super) outcome: &'static str,
    pub(super) agent_id: Uuid,
    pub(super) tick_id: Option<String>,
    pub(super) event_id: Option<i64>,
    pub(super) mood_label: Option<String>,
    pub(super) valence: Option<f32>,
    pub(super) arousal: Option<f32>,
}

#[derive(Deserialize)]
pub(super) struct EventsQuery {
    pub(super) agent_id: Option<Uuid>,
    pub(super) limit: Option<u32>,
    pub(super) after_id: Option<i64>,
}

#[derive(Serialize)]
pub(super) struct EventsResponse {
    pub(super) items: Vec<EventItemResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) next_after_id: Option<i64>,
}

#[derive(Serialize, Clone)]
pub(super) struct EventItemResponse {
    pub(super) id: i64,
    pub(super) agent_id: Option<Uuid>,
    pub(super) event_type: String,
    pub(super) description: String,
    pub(super) payload: String,
    pub(super) occurred_at: String,
}

#[derive(Serialize)]
pub(super) struct AgentStateResponse {
    pub(super) agent_id: Uuid,
    pub(super) mood_label: String,
    pub(super) valence: f32,
    pub(super) arousal: f32,
    pub(super) updated_at: String,
}

#[derive(Deserialize)]
pub(super) struct AgentInspectorQuery {
    pub(super) events_limit: Option<u32>,
    pub(super) messages_limit: Option<u32>,
    pub(super) relationships_limit: Option<u32>,
    pub(super) memories_limit: Option<u32>,
    pub(super) timeline_limit: Option<u32>,
    pub(super) recall_query: Option<String>,
    pub(super) recall_top_k: Option<u32>,
}

#[derive(Serialize)]
pub(super) struct AgentInspectorResponse {
    pub(super) agent: AgentInspectorAgentResponse,
    pub(super) state: Option<AgentStateResponse>,
    pub(super) recent_events: Vec<EventItemResponse>,
    pub(super) recent_messages: Vec<MessageItemResponse>,
    pub(super) recent_relationships: Vec<RelationshipItemResponse>,
    pub(super) relationship_timeline: Vec<RelationshipTimelineItemResponse>,
    pub(super) recent_memories: Vec<InspectorMemoryItemResponse>,
    pub(super) recall: Option<AgentInspectorRecallResponse>,
    pub(super) summary: AgentInspectorSummaryResponse,
}

#[derive(Serialize)]
pub(super) struct AgentInspectorAgentResponse {
    pub(super) id: Uuid,
    pub(super) name: String,
    pub(super) avatar_url: Option<String>,
    pub(super) personality_json: Value,
    pub(super) created_at: String,
}

#[derive(Serialize)]
pub(super) struct InspectorMemoryItemResponse {
    pub(super) memory_id: i64,
    pub(super) content: String,
    pub(super) summary: Option<String>,
    pub(super) importance: f32,
    pub(super) is_summary: bool,
    pub(super) embedding_status: String,
    pub(super) created_at: String,
}

#[derive(Serialize)]
pub(super) struct AgentInspectorRecallResponse {
    pub(super) query: String,
    pub(super) top_k: u32,
    pub(super) items: Vec<RecallItemResponse>,
}

#[derive(Serialize)]
pub(super) struct AgentInspectorSummaryResponse {
    pub(super) events_count: usize,
    pub(super) messages_count: usize,
    pub(super) relationships_count: usize,
    pub(super) timeline_count: usize,
    pub(super) memories_count: usize,
}

#[derive(Serialize, Clone)]
pub(super) struct ApiErrorResponse {
    pub(super) error: &'static str,
    pub(super) message: String,
}

#[derive(Deserialize)]
pub(super) struct AuthRegisterRequest {
    pub(super) name: String,
    pub(super) email: String,
    pub(super) password: String,
}

#[derive(Deserialize)]
pub(super) struct AuthLoginRequest {
    pub(super) email: String,
    pub(super) password: String,
}

#[derive(Deserialize)]
pub(super) struct AuthRefreshRequest {
    pub(super) refresh_token: Option<String>,
    #[serde(alias = "refreshToken")]
    pub(super) refresh_token_alias: Option<String>,
}

#[derive(Serialize)]
pub(super) struct AuthSessionResponse {
    pub(super) user: AuthUserResponse,
    pub(super) tokens: AuthTokensResponse,
}

#[derive(Serialize)]
pub(super) struct AuthUserResponse {
    pub(super) id: Uuid,
    pub(super) email: String,
    pub(super) name: String,
    pub(super) created_at: String,
}

#[derive(Serialize)]
pub(super) struct AuthTokensResponse {
    pub(super) access_token: String,
    pub(super) refresh_token: String,
    pub(super) access_expires_at: String,
    pub(super) refresh_expires_at: String,
    pub(super) token_type: &'static str,
}

#[derive(Clone)]
pub(super) struct AuthUserRecord {
    pub(super) id: Uuid,
    pub(super) email: String,
    pub(super) name: String,
    pub(super) password_hash: String,
    pub(super) created_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Clone)]
pub(super) struct JwtClaims {
    pub(super) sub: String,
    pub(super) email: String,
    pub(super) token_use: String,
    pub(super) exp: usize,
    pub(super) iat: usize,
    pub(super) nbf: usize,
    pub(super) jti: String,
}

#[derive(Clone)]
pub(super) struct AuthenticatedUser {
    pub(super) user_id: Uuid,
}

#[derive(Deserialize)]
pub(super) struct AppendMemoryRequest {
    pub(super) content: String,
    pub(super) importance: Option<f32>,
}

#[derive(Serialize)]
pub(super) struct AppendMemoryResponse {
    pub(super) memory_id: i64,
    pub(super) embedding_status: String,
}

#[derive(Deserialize)]
pub(super) struct RecallQuery {
    pub(super) query: String,
    pub(super) top_k: Option<u32>,
}

#[derive(Serialize)]
pub(super) struct RecallResponse {
    pub(super) items: Vec<RecallItemResponse>,
}

#[derive(Serialize)]
pub(super) struct RecallItemResponse {
    pub(super) memory_id: i64,
    pub(super) score: f32,
    pub(super) content: String,
    pub(super) summary: Option<String>,
    pub(super) importance: f32,
    pub(super) created_at: String,
}

#[derive(Deserialize)]
pub(super) struct SummarizeMemoryRequest {
    pub(super) max_active: Option<u32>,
    pub(super) batch_size: Option<u32>,
}

#[derive(Serialize)]
pub(super) struct SummarizeMemoryResponse {
    pub(super) created_summary: bool,
    pub(super) source_count: u32,
    pub(super) summary_entry_id: Option<i64>,
}

#[derive(Deserialize)]
pub(super) struct ProcessEmbeddingsRequest {
    pub(super) limit: Option<u32>,
}

#[derive(Serialize)]
pub(super) struct ProcessEmbeddingsResponse {
    pub(super) processed: u32,
    pub(super) succeeded: u32,
    pub(super) failed: u32,
    pub(super) retried: u32,
    pub(super) dead_lettered: u32,
}

#[derive(Deserialize)]
pub(super) struct DeadLetterQuery {
    pub(super) limit: Option<u32>,
}

#[derive(Serialize)]
pub(super) struct DeadLetterEmbeddingsResponse {
    pub(super) items: Vec<DeadLetterEmbeddingItemResponse>,
}

#[derive(Serialize)]
pub(super) struct DeadLetterEmbeddingItemResponse {
    pub(super) memory_id: i64,
    pub(super) agent_id: Uuid,
    pub(super) content: String,
    pub(super) summary: Option<String>,
    pub(super) importance: f32,
    pub(super) created_at: String,
    pub(super) embedding_status: String,
}

#[derive(Serialize)]
pub(super) struct RequeueDeadLetterResponse {
    pub(super) memory_id: i64,
    pub(super) requeued: bool,
}

#[derive(Serialize)]
pub(super) struct SimulationTimeScaleResponse {
    pub(super) time_scale: f32,
    pub(super) updated_at: String,
}

#[derive(Deserialize)]
pub(super) struct SetSimulationTimeScaleRequest {
    pub(super) time_scale: f32,
}

#[derive(Deserialize)]
pub(super) struct CreateInterventionRequest {
    pub(super) admin_user_id: String,
    pub(super) action: InterventionActionRequest,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum InterventionActionRequest {
    TriggerTick {
        agent_id: Uuid,
        tick_id: Option<String>,
    },
    AppendMemory {
        agent_id: Uuid,
        content: String,
        importance: Option<f32>,
    },
    SendMessage {
        sender_agent_id: Uuid,
        receiver_agent_id: Uuid,
        content: String,
    },
    AppendEvent {
        agent_id: Option<Uuid>,
        event_type: String,
        description: String,
        payload_json: Option<Value>,
    },
    SetTimeScale {
        time_scale: f32,
    },
}

impl InterventionActionRequest {
    pub(super) fn action_type(&self) -> &'static str {
        match self {
            Self::TriggerTick { .. } => "trigger_tick",
            Self::AppendMemory { .. } => "append_memory",
            Self::SendMessage { .. } => "send_message",
            Self::AppendEvent { .. } => "append_event",
            Self::SetTimeScale { .. } => "set_time_scale",
        }
    }

    pub(super) fn payload_json(&self) -> Value {
        match self {
            Self::TriggerTick { agent_id, tick_id } => json!({
                "agent_id": agent_id,
                "tick_id": tick_id,
            }),
            Self::AppendMemory {
                agent_id,
                content,
                importance,
            } => json!({
                "agent_id": agent_id,
                "content": content,
                "importance": importance,
            }),
            Self::SendMessage {
                sender_agent_id,
                receiver_agent_id,
                content,
            } => json!({
                "sender_agent_id": sender_agent_id,
                "receiver_agent_id": receiver_agent_id,
                "content": content,
            }),
            Self::AppendEvent {
                agent_id,
                event_type,
                description,
                payload_json,
            } => json!({
                "agent_id": agent_id,
                "event_type": event_type,
                "description": description,
                "payload_json": payload_json,
            }),
            Self::SetTimeScale { time_scale } => json!({
                "time_scale": time_scale,
            }),
        }
    }
}

#[derive(Deserialize)]
pub(super) struct InterventionListQuery {
    pub(super) limit: Option<u32>,
}

#[derive(Serialize)]
pub(super) struct InterventionsResponse {
    pub(super) items: Vec<InterventionItemResponse>,
}

#[derive(Serialize)]
pub(super) struct CreateInterventionResponse {
    pub(super) intervention: InterventionItemResponse,
    pub(super) effect: InterventionEffectResponse,
}

#[derive(Serialize)]
pub(super) struct InterventionItemResponse {
    pub(super) id: i64,
    pub(super) admin_user_id: String,
    pub(super) action_type: String,
    pub(super) payload_json: Value,
    pub(super) result_status: String,
    pub(super) created_at: String,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum InterventionEffectResponse {
    Tick {
        agent_id: Uuid,
        outcome: String,
        tick_id: Option<String>,
        event_id: Option<i64>,
        mood_label: Option<String>,
        valence: Option<f32>,
        arousal: Option<f32>,
    },
    Memory {
        agent_id: Uuid,
        memory_id: i64,
        embedding_status: String,
    },
    Message {
        message_id: i64,
        status: String,
    },
    Event {
        event_id: i64,
        event_type: String,
    },
    TimeScale {
        time_scale: f32,
        updated_at: String,
    },
}

#[derive(Deserialize)]
pub(super) struct CreateMessageRequest {
    pub(super) sender_agent_id: Uuid,
    pub(super) content: String,
}

#[derive(Serialize)]
pub(super) struct CreateMessageResponse {
    pub(super) message_id: i64,
    pub(super) status: String,
}

#[derive(Deserialize)]
pub(super) struct MessageListQuery {
    pub(super) limit: Option<u32>,
}

#[derive(Serialize)]
pub(super) struct AgentMessagesResponse {
    pub(super) items: Vec<MessageItemResponse>,
}

#[derive(Serialize)]
pub(super) struct MessageItemResponse {
    pub(super) id: i64,
    pub(super) sender_type: String,
    pub(super) sender_id: Option<Uuid>,
    pub(super) receiver_agent_id: Uuid,
    pub(super) content: String,
    pub(super) status: String,
    pub(super) created_at: String,
}

#[derive(Deserialize)]
pub(super) struct RelationshipListQuery {
    pub(super) limit: Option<u32>,
}

#[derive(Deserialize)]
pub(super) struct RelationshipTimelineQuery {
    pub(super) limit: Option<u32>,
}

#[derive(Deserialize)]
pub(super) struct RelationshipGraphQuery {
    pub(super) agent_id: Option<Uuid>,
    pub(super) limit_edges: Option<u32>,
}

#[derive(Serialize)]
pub(super) struct AgentRelationshipsResponse {
    pub(super) items: Vec<RelationshipItemResponse>,
}

#[derive(Serialize)]
pub(super) struct AgentRelationshipTimelineResponse {
    pub(super) agent_id: Uuid,
    pub(super) items: Vec<RelationshipTimelineItemResponse>,
}

#[derive(Serialize, Clone)]
pub(super) struct RelationshipItemResponse {
    pub(super) id: i64,
    pub(super) agent_a: Uuid,
    pub(super) agent_b: Uuid,
    pub(super) affinity_score: f32,
    pub(super) history_summary: String,
    pub(super) last_interaction_at: Option<String>,
    pub(super) created_at: String,
}

#[derive(Serialize, Clone)]
pub(super) struct RelationshipTimelineItemResponse {
    pub(super) message_id: i64,
    pub(super) direction: String,
    pub(super) counterpart_agent_id: Option<Uuid>,
    pub(super) counterpart_name: Option<String>,
    pub(super) counterpart_avatar_url: Option<String>,
    pub(super) content: String,
    pub(super) status: String,
    pub(super) created_at: String,
    pub(super) relationship: Option<RelationshipItemResponse>,
}

#[derive(Serialize, Clone)]
pub(super) struct RelationshipGraphNodeResponse {
    pub(super) agent_id: Uuid,
    pub(super) name: String,
    pub(super) avatar_url: Option<String>,
}

#[derive(Serialize, Clone)]
pub(super) struct RelationshipGraphResponse {
    pub(super) nodes: Vec<RelationshipGraphNodeResponse>,
    pub(super) edges: Vec<RelationshipItemResponse>,
}

#[derive(Deserialize)]
pub(super) struct WsEventsQuery {
    pub(super) agent_id: Option<Uuid>,
    pub(super) snapshot_limit: Option<u32>,
}

#[derive(Deserialize)]
pub(super) struct WsRelationshipsQuery {
    pub(super) agent_id: Option<Uuid>,
    pub(super) snapshot_limit: Option<u32>,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum WsServerEvent {
    Snapshot {
        items: Vec<EventItemResponse>,
    },
    TickSkipped {
        agent_id: Uuid,
        reason: String,
        tick_id: Option<String>,
    },
    EventAppended {
        item: EventItemResponse,
    },
    RelationshipUpdated {
        edge: RelationshipItemResponse,
    },
    Error {
        message: String,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum WsRelationshipServerEvent {
    Snapshot { graph: RelationshipGraphResponse },
    EdgeUpdated { edge: RelationshipItemResponse },
    Error { message: String },
}
