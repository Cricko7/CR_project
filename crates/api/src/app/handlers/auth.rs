use super::*;

pub(crate) async fn auth_register(
    State(state): State<AuthState>,
    Json(payload): Json<AuthRegisterRequest>,
) -> Result<(StatusCode, Json<AuthSessionResponse>), (StatusCode, Json<ApiErrorResponse>)> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_name",
                message: "name must not be empty".to_owned(),
            }),
        ));
    }

    let email = normalize_email(&payload.email)?;
    validate_password(&payload.password)?;
    let password_hash = hash_password(&payload.password)?;

    let user = state
        .repository
        .create_user(name, &email, &password_hash)
        .await
        .map_err(|error| match error {
            AuthRepoError::EmailTaken => (
                StatusCode::CONFLICT,
                Json(ApiErrorResponse {
                    error: "email_taken",
                    message: "user with this email already exists".to_owned(),
                }),
            ),
            AuthRepoError::Other(error) => internal_error(
                "auth_register_failed",
                format!("failed to create user: {error}"),
            ),
        })?;

    let tokens = state.manager.issue_session_tokens(&user)?;
    Ok((
        StatusCode::CREATED,
        Json(AuthSessionResponse {
            user: map_auth_user(&user),
            tokens,
        }),
    ))
}

pub(crate) async fn auth_login(
    State(state): State<AuthState>,
    Json(payload): Json<AuthLoginRequest>,
) -> Result<Json<AuthSessionResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let email = normalize_email(&payload.email)?;
    let Some(user) = state
        .repository
        .find_by_email(&email)
        .await
        .map_err(|error| {
            internal_error(
                "auth_login_failed",
                format!("failed to read auth user: {error}"),
            )
        })?
    else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResponse {
                error: "invalid_credentials",
                message: "invalid email or password".to_owned(),
            }),
        ));
    };

    verify_password(&payload.password, &user.password_hash)?;
    let tokens = state.manager.issue_session_tokens(&user)?;

    Ok(Json(AuthSessionResponse {
        user: map_auth_user(&user),
        tokens,
    }))
}

pub(crate) async fn auth_refresh(
    State(state): State<AuthState>,
    Json(payload): Json<AuthRefreshRequest>,
) -> Result<Json<AuthSessionResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let refresh_token = payload
        .refresh_token
        .or(payload.refresh_token_alias)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResponse {
                    error: "missing_refresh_token",
                    message: "refresh_token is required".to_owned(),
                }),
            )
        })?;

    let authenticated = state.manager.validate_token(&refresh_token, "refresh")?;
    let Some(user) = state
        .repository
        .find_by_id(authenticated.user_id)
        .await
        .map_err(|error| {
            internal_error(
                "auth_refresh_failed",
                format!("failed to read refresh user: {error}"),
            )
        })?
    else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResponse {
                error: "invalid_token",
                message: "refresh token user does not exist".to_owned(),
            }),
        ));
    };

    let tokens = state.manager.issue_session_tokens(&user)?;
    Ok(Json(AuthSessionResponse {
        user: map_auth_user(&user),
        tokens,
    }))
}
