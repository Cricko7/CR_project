use super::*;

pub(super) struct AuthManager {
    pub(super) jwt_secret: String,
    pub(super) access_ttl: Duration,
    pub(super) refresh_ttl: Duration,
}

impl AuthManager {
    pub(super) fn new(jwt_secret: String, access_ttl: Duration, refresh_ttl: Duration) -> Self {
        Self {
            jwt_secret,
            access_ttl,
            refresh_ttl,
        }
    }
    pub(super) fn issue_session_tokens(
        &self,
        user: &AuthUserRecord,
    ) -> Result<AuthTokensResponse, (StatusCode, Json<ApiErrorResponse>)> {
        let access = self.issue_token(user, "access", self.access_ttl)?;
        let refresh = self.issue_token(user, "refresh", self.refresh_ttl)?;

        Ok(AuthTokensResponse {
            access_token: access.token,
            refresh_token: refresh.token,
            access_expires_at: access.expires_at.to_rfc3339(),
            refresh_expires_at: refresh.expires_at.to_rfc3339(),
            token_type: "Bearer",
        })
    }

    fn issue_token(
        &self,
        user: &AuthUserRecord,
        token_use: &'static str,
        ttl: Duration,
    ) -> Result<IssuedToken, (StatusCode, Json<ApiErrorResponse>)> {
        let now = Utc::now();
        let expires_at = now + chrono::Duration::seconds(ttl.as_secs() as i64);
        let claims = JwtClaims {
            sub: user.id.to_string(),
            email: user.email.clone(),
            token_use: token_use.to_owned(),
            exp: expires_at.timestamp().max(0) as usize,
            iat: now.timestamp().max(0) as usize,
            nbf: now.timestamp().max(0) as usize,
            jti: Uuid::new_v4().to_string(),
        };

        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )
        .map_err(|error| {
            internal_error(
                "jwt_issue_failed",
                format!("failed to issue jwt token: {error}"),
            )
        })?;

        Ok(IssuedToken { token, expires_at })
    }
    pub(super) fn validate_token(
        &self,
        token: &str,
        expected_use: &'static str,
    ) -> Result<AuthenticatedUser, (StatusCode, Json<ApiErrorResponse>)> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_nbf = true;
        validation.leeway = 5;

        let decoded = decode::<JwtClaims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &validation,
        )
        .map_err(|_| {
            (
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResponse {
                    error: "invalid_token",
                    message: "token is invalid or expired".to_owned(),
                }),
            )
        })?;

        if decoded.claims.token_use != expected_use {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResponse {
                    error: "invalid_token_use",
                    message: format!("expected `{expected_use}` token"),
                }),
            ));
        }

        let user_id = Uuid::parse_str(&decoded.claims.sub).map_err(|_| {
            (
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResponse {
                    error: "invalid_token_subject",
                    message: "token subject is invalid".to_owned(),
                }),
            )
        })?;

        Ok(AuthenticatedUser { user_id })
    }
}

pub(super) struct IssuedToken {
    pub(super) token: String,
    pub(super) expires_at: DateTime<Utc>,
}

#[derive(Clone)]
pub(super) struct PostgresAuthRepository {
    pub(super) pool: sqlx::PgPool,
}

impl PostgresAuthRepository {
    pub(super) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
    pub(super) async fn ensure_schema(&self) -> Result<(), anyhow::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS auth_users (
                id UUID PRIMARY KEY,
                email TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                password_hash TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&self.pool)
        .await
        .context("failed to ensure auth_users table")?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_auth_users_email ON auth_users (email)")
            .execute(&self.pool)
            .await
            .context("failed to ensure idx_auth_users_email index")?;

        Ok(())
    }
    pub(super) async fn create_user(
        &self,
        name: &str,
        email: &str,
        password_hash: &str,
    ) -> Result<AuthUserRecord, AuthRepoError> {
        let id = Uuid::new_v4();
        let row = sqlx::query(
            "INSERT INTO auth_users (id, email, name, password_hash)
             VALUES ($1, $2, $3, $4)
             RETURNING id, email, name, password_hash, created_at",
        )
        .bind(id)
        .bind(email)
        .bind(name)
        .bind(password_hash)
        .fetch_one(&self.pool)
        .await
        .map_err(map_auth_repo_error)?;

        Ok(AuthUserRecord {
            id: row.get("id"),
            email: row.get("email"),
            name: row.get("name"),
            password_hash: row.get("password_hash"),
            created_at: row.get("created_at"),
        })
    }
    pub(super) async fn find_by_email(
        &self,
        email: &str,
    ) -> Result<Option<AuthUserRecord>, anyhow::Error> {
        let row = sqlx::query(
            "SELECT id, email, name, password_hash, created_at
             FROM auth_users
             WHERE email = $1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .context("failed to read auth user by email")?;

        Ok(row.map(|row| AuthUserRecord {
            id: row.get("id"),
            email: row.get("email"),
            name: row.get("name"),
            password_hash: row.get("password_hash"),
            created_at: row.get("created_at"),
        }))
    }
    pub(super) async fn find_by_id(
        &self,
        id: Uuid,
    ) -> Result<Option<AuthUserRecord>, anyhow::Error> {
        let row = sqlx::query(
            "SELECT id, email, name, password_hash, created_at
             FROM auth_users
             WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("failed to read auth user by id")?;

        Ok(row.map(|row| AuthUserRecord {
            id: row.get("id"),
            email: row.get("email"),
            name: row.get("name"),
            password_hash: row.get("password_hash"),
            created_at: row.get("created_at"),
        }))
    }
}

pub(super) enum AuthRepoError {
    EmailTaken,
    Other(anyhow::Error),
}

pub(super) fn map_auth_repo_error(error: sqlx::Error) -> AuthRepoError {
    if let sqlx::Error::Database(database_error) = &error
        && database_error.code().as_deref() == Some("23505")
    {
        return AuthRepoError::EmailTaken;
    }

    AuthRepoError::Other(anyhow::Error::from(error))
}
