use super::*;

pub(crate) async fn auth_middleware(
    State(state): State<AuthState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if is_public_path(path) {
        return next.run(request).await;
    }

    let token = read_bearer_token(request.headers()).or_else(|| {
        if path.starts_with("/ws/") {
            return extract_access_token_from_query(request.uri());
        }
        None
    });

    let Some(token) = token else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResponse {
                error: "missing_auth_token",
                message: "bearer access token is required".to_owned(),
            }),
        )
            .into_response();
    };

    match state.manager.validate_token(&token, "access") {
        Ok(authenticated) => {
            request.extensions_mut().insert(authenticated);
            next.run(request).await
        }
        Err(error) => error.into_response(),
    }
}

pub(crate) async fn rate_limit_middleware(
    State(state): State<RateLimitState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if is_rate_limit_exempt(request.uri().path()) {
        return next.run(request).await;
    }

    let ip = extract_client_ip(&request);
    if state.limiter.allow(&ip).await {
        return next.run(request).await;
    }

    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(ApiErrorResponse {
            error: "rate_limit_exceeded",
            message: "too many requests from your ip, please retry later".to_owned(),
        }),
    )
        .into_response()
}
