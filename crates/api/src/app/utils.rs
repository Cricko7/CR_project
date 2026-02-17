use super::*;

pub(super) fn map_recall_item(item: MemoryRecallItem) -> RecallItemResponse {
    RecallItemResponse {
        memory_id: item.memory.id,
        score: item.score,
        content: item.memory.content,
        summary: item.memory.summary,
        importance: item.memory.importance,
        created_at: item.memory.created_at.to_rfc3339(),
    }
}

pub(super) fn map_dead_letter_memory(memory: MemoryEntryRecord) -> DeadLetterEmbeddingItemResponse {
    DeadLetterEmbeddingItemResponse {
        memory_id: memory.id,
        agent_id: memory.agent_id,
        content: memory.content,
        summary: memory.summary,
        importance: memory.importance,
        created_at: memory.created_at.to_rfc3339(),
        embedding_status: memory.embedding_status,
    }
}

pub(super) fn map_inspector_memory(memory: MemoryEntryRecord) -> InspectorMemoryItemResponse {
    InspectorMemoryItemResponse {
        memory_id: memory.id,
        content: memory.content,
        summary: memory.summary,
        importance: memory.importance,
        is_summary: memory.is_summary,
        embedding_status: memory.embedding_status,
        created_at: memory.created_at.to_rfc3339(),
    }
}

pub(super) fn map_simulation_time_scale_record(
    record: SimulationTimeScaleRecord,
) -> SimulationTimeScaleResponse {
    SimulationTimeScaleResponse {
        time_scale: record.time_scale,
        updated_at: record.updated_at.to_rfc3339(),
    }
}

pub(super) fn map_intervention_record(record: InterventionRecord) -> InterventionItemResponse {
    InterventionItemResponse {
        id: record.id,
        admin_user_id: record.admin_user_id,
        action_type: record.action_type,
        payload_json: record.payload_json,
        result_status: record.result_status,
        created_at: record.created_at.to_rfc3339(),
    }
}

pub(super) fn map_message_record(message: MessageRecord) -> MessageItemResponse {
    MessageItemResponse {
        id: message.id,
        sender_type: message.sender_type,
        sender_id: message.sender_id,
        receiver_agent_id: message.receiver_agent_id,
        content: message.content,
        status: message.status,
        created_at: message.created_at.to_rfc3339(),
    }
}

pub(super) fn map_relationship_record(record: RelationshipRecord) -> RelationshipItemResponse {
    RelationshipItemResponse {
        id: record.id,
        agent_a: record.agent_a,
        agent_b: record.agent_b,
        affinity_score: record.affinity_score,
        history_summary: record.history_summary,
        last_interaction_at: record.last_interaction_at.map(|value| value.to_rfc3339()),
        created_at: record.created_at.to_rfc3339(),
    }
}

pub(super) fn map_auth_user(user: &AuthUserRecord) -> AuthUserResponse {
    AuthUserResponse {
        id: user.id,
        email: user.email.clone(),
        name: user.name.clone(),
        created_at: user.created_at.to_rfc3339(),
    }
}

pub(super) fn normalize_email(raw: &str) -> Result<String, (StatusCode, Json<ApiErrorResponse>)> {
    let normalized = raw.trim().to_lowercase();
    if normalized.is_empty() || !normalized.contains('@') {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_email",
                message: "email must be a valid address".to_owned(),
            }),
        ));
    }
    Ok(normalized)
}

pub(super) fn validate_password(
    password: &str,
) -> Result<(), (StatusCode, Json<ApiErrorResponse>)> {
    if password.len() < MIN_PASSWORD_LENGTH {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_password",
                message: format!("password must contain at least {MIN_PASSWORD_LENGTH} characters"),
            }),
        ));
    }
    Ok(())
}

pub(super) fn hash_password(
    password: &str,
) -> Result<String, (StatusCode, Json<ApiErrorResponse>)> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| {
            internal_error(
                "password_hash_failed",
                format!("failed to hash password: {error}"),
            )
        })?;
    Ok(hash.to_string())
}

pub(super) fn verify_password(
    password: &str,
    password_hash: &str,
) -> Result<(), (StatusCode, Json<ApiErrorResponse>)> {
    let parsed_hash = PasswordHash::new(password_hash).map_err(|error| {
        internal_error(
            "password_hash_invalid",
            format!("stored password hash is invalid: {error}"),
        )
    })?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .map_err(|_| {
            (
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResponse {
                    error: "invalid_credentials",
                    message: "invalid email or password".to_owned(),
                }),
            )
        })
}

pub(super) fn is_public_path(path: &str) -> bool {
    matches!(
        path,
        "/health" | "/livez" | "/auth/register" | "/auth/login" | "/auth/refresh"
    )
}

pub(super) fn is_rate_limit_exempt(path: &str) -> bool {
    matches!(path, "/health" | "/livez")
}

pub(super) fn read_bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let raw = headers.get(AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, token) = raw.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    Some(token.to_owned())
}

pub(super) fn extract_access_token_from_query(uri: &Uri) -> Option<String> {
    let query = uri.query()?;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        if key == "access_token" && !value.trim().is_empty() {
            return Some(value.to_owned());
        }
    }
    None
}

pub(super) fn extract_client_ip(request: &Request<Body>) -> String {
    if let Some(ConnectInfo(addr)) = request.extensions().get::<ConnectInfo<SocketAddr>>() {
        return addr.ip().to_string();
    }

    if let Some(value) = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|header| header.to_str().ok())
        .and_then(|header| header.split(',').next())
    {
        let candidate = value.trim();
        if !candidate.is_empty() {
            return candidate.to_owned();
        }
    }

    if let Some(value) = request
        .headers()
        .get("x-real-ip")
        .and_then(|header| header.to_str().ok())
    {
        let candidate = value.trim();
        if !candidate.is_empty() {
            return candidate.to_owned();
        }
    }
    "unknown".to_owned()
}

pub(super) fn validate_time_scale(
    time_scale: f32,
) -> Result<f32, (StatusCode, Json<ApiErrorResponse>)> {
    if !time_scale.is_finite() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_time_scale",
                message: "time_scale must be a finite number".to_owned(),
            }),
        ));
    }

    if !(MIN_SIMULATION_TIME_SCALE..=MAX_SIMULATION_TIME_SCALE).contains(&time_scale) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_time_scale",
                message: format!(
                    "time_scale must be in range [{MIN_SIMULATION_TIME_SCALE}, {MAX_SIMULATION_TIME_SCALE}]",
                ),
            }),
        ));
    }

    Ok(time_scale)
}

pub(super) fn internal_error(
    error: &'static str,
    message: String,
) -> (StatusCode, Json<ApiErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResponse { error, message }),
    )
}
