use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config;

const TOKEN_EXPIRY_SECS: i64 = 86400; // 24 hours

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: Uuid,       // user_id
    pub email: String,
    pub permissions: Vec<String>,
    pub exp: i64,
    pub iat: i64,
}

#[derive(Debug)]
pub enum JwtError {
    EncodingFailed(String),
    InvalidToken(String),
    Expired,
}

impl std::fmt::Display for JwtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JwtError::EncodingFailed(e) => write!(f, "Token encoding failed: {e}"),
            JwtError::InvalidToken(e) => write!(f, "Invalid token: {e}"),
            JwtError::Expired => write!(f, "Token expired"),
        }
    }
}

pub fn create_token(
    user_id: Uuid,
    email: &str,
    permissions: Vec<String>,
) -> Result<String, JwtError> {
    let cfg = config::get();
    let now = chrono::Utc::now().timestamp();

    let claims = Claims {
        sub: user_id,
        email: email.to_string(),
        permissions,
        exp: now + TOKEN_EXPIRY_SECS,
        iat: now,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(cfg.jwt_secret.as_bytes()),
    )
    .map_err(|e| JwtError::EncodingFailed(e.to_string()))
}

pub fn validate_token(token: &str) -> Result<Claims, JwtError> {
    let cfg = config::get();

    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(cfg.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => JwtError::Expired,
        _ => JwtError::InvalidToken(e.to_string()),
    })?;

    Ok(token_data.claims)
}
