use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Role {
    pub id: Uuid,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Permission {
    pub resource: String,
    pub action: String,
}

#[derive(Debug)]
pub enum RbacError {
    RoleNotFound(String),
    DbError(sqlx::Error),
}

impl std::fmt::Display for RbacError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RbacError::RoleNotFound(name) => write!(f, "Role not found: {name}"),
            RbacError::DbError(e) => write!(f, "Database error: {e}"),
        }
    }
}

impl From<sqlx::Error> for RbacError {
    fn from(e: sqlx::Error) -> Self {
        RbacError::DbError(e)
    }
}

pub struct RbacService {
    pool: PgPool,
}

impl RbacService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get all roles assigned to a user.
    pub async fn get_user_roles(&self, user_id: Uuid) -> Result<Vec<Role>, RbacError> {
        let roles = sqlx::query_as::<_, Role>(
            "SELECT r.id, r.name, r.description
             FROM roles r
             JOIN user_roles ur ON ur.role_id = r.id
             WHERE ur.user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(roles)
    }

    /// Get role names for a user (for JWT claims).
    pub async fn get_user_role_names(&self, user_id: Uuid) -> Result<Vec<String>, RbacError> {
        let names = sqlx::query_scalar::<_, String>(
            "SELECT r.name
             FROM roles r
             JOIN user_roles ur ON ur.role_id = r.id
             WHERE ur.user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(names)
    }

    /// Get all permissions for a user across all their roles.
    pub async fn get_user_permissions(&self, user_id: Uuid) -> Result<Vec<Permission>, RbacError> {
        let perms = sqlx::query_as::<_, Permission>(
            "SELECT DISTINCT p.resource, p.action
             FROM permissions p
             JOIN user_roles ur ON ur.role_id = p.role_id
             WHERE ur.user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(perms)
    }

    /// Check if a user has a specific role.
    pub async fn has_role(&self, user_id: Uuid, role_name: &str) -> Result<bool, RbacError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM user_roles ur
             JOIN roles r ON r.id = ur.role_id
             WHERE ur.user_id = $1 AND r.name = $2",
        )
        .bind(user_id)
        .bind(role_name)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    /// Check if a user has permission for a resource+action.
    pub async fn has_permission(
        &self,
        user_id: Uuid,
        resource: &str,
        action: &str,
    ) -> Result<bool, RbacError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM permissions p
             JOIN user_roles ur ON ur.role_id = p.role_id
             WHERE ur.user_id = $1
             AND (p.resource = $2 OR p.resource = '*')
             AND (p.action = $3 OR p.action = '*')",
        )
        .bind(user_id)
        .bind(resource)
        .bind(action)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    /// Assign a role to a user by role name.
    pub async fn assign_role(&self, user_id: Uuid, role_name: &str) -> Result<(), RbacError> {
        let role_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM roles WHERE name = $1",
        )
        .bind(role_name)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| RbacError::RoleNotFound(role_name.to_string()))?;

        sqlx::query(
            "INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)
             ON CONFLICT (user_id, role_id) DO NOTHING",
        )
        .bind(user_id)
        .bind(role_id)
        .execute(&self.pool)
        .await?;

        tracing::info!(user_id = %user_id, role = role_name, "role_assigned");
        Ok(())
    }

    /// List all available roles.
    pub async fn list_roles(&self) -> Result<Vec<Role>, RbacError> {
        let roles = sqlx::query_as::<_, Role>(
            "SELECT id, name, description FROM roles ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(roles)
    }
}
