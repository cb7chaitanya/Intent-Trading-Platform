use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::accounts::service::AccountService;

use super::model::{AuthResponse, LoginRequest, RegisterRequest, User};
use super::repository::UserRepository;

#[derive(Debug)]
pub enum UserError {
    EmailTaken,
    WeakPassword(Vec<String>),
    InvalidCredentials,
    HashError(String),
    DbError(sqlx::Error),
    AccountError(String),
}

impl std::fmt::Display for UserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserError::EmailTaken => write!(f, "Email already registered"),
            UserError::WeakPassword(errs) => write!(f, "Weak password: {}", errs.join("; ")),
            UserError::InvalidCredentials => write!(f, "Invalid email or password"),
            UserError::HashError(e) => write!(f, "Password hash error: {e}"),
            UserError::DbError(e) => write!(f, "Database error: {e}"),
            UserError::AccountError(e) => write!(f, "Account creation error: {e}"),
        }
    }
}

impl From<sqlx::Error> for UserError {
    fn from(e: sqlx::Error) -> Self {
        UserError::DbError(e)
    }
}

pub struct UserService {
    repo: UserRepository,
    account_service: Arc<AccountService>,
}

impl UserService {
    pub fn new(repo: UserRepository, account_service: Arc<AccountService>) -> Self {
        Self {
            repo,
            account_service,
        }
    }

    pub async fn register(&self, req: RegisterRequest) -> Result<AuthResponse, UserError> {
        if let Err(violations) = super::password::validate(&req.password, &req.email) {
            return Err(UserError::WeakPassword(violations));
        }

        if self.repo.find_by_email(&req.email).await?.is_some() {
            return Err(UserError::EmailTaken);
        }

        let password_hash =
            bcrypt::hash(&req.password, bcrypt::DEFAULT_COST).map_err(|e| UserError::HashError(e.to_string()))?;

        let now = Utc::now();
        let user = User {
            id: Uuid::new_v4(),
            email: req.email,
            password_hash,
            created_at: now,
            updated_at: now,
        };

        self.repo.insert(&user).await?;

        // Create default Spot account
        self.account_service
            .create_default_account(user.id)
            .await
            .map_err(|e| UserError::AccountError(e.to_string()))?;

        Ok(AuthResponse {
            user_id: user.id,
            email: user.email,
            message: "Registration successful".to_string(),
        })
    }

    pub async fn login(&self, req: LoginRequest) -> Result<AuthResponse, UserError> {
        let user = self
            .repo
            .find_by_email(&req.email)
            .await?
            .ok_or(UserError::InvalidCredentials)?;

        let valid =
            bcrypt::verify(&req.password, &user.password_hash).map_err(|e| UserError::HashError(e.to_string()))?;

        if !valid {
            return Err(UserError::InvalidCredentials);
        }

        Ok(AuthResponse {
            user_id: user.id,
            email: user.email,
            message: "Login successful".to_string(),
        })
    }

    pub async fn get_user(&self, id: Uuid) -> Result<Option<User>, UserError> {
        Ok(self.repo.find_by_id(id).await?)
    }
}
