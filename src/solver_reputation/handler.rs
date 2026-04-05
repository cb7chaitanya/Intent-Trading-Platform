use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use super::model::{
    RegisterSolverRequest, RegisterSolverResponse, SolverDashboard, SolverPublic,
    TopSolversQuery, UpdateSolverRequest,
};
use super::service::{SolverError, SolverReputationService};
use super::stats::{LeaderboardEntry, SolverStats};

// ── Registration (public) ──────────────────────────────

pub async fn register_solver(
    State(svc): State<Arc<SolverReputationService>>,
    Json(req): Json<RegisterSolverRequest>,
) -> Result<(StatusCode, Json<RegisterSolverResponse>), (StatusCode, String)> {
    let name = req.name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Name must be 1-64 characters".to_string(),
        ));
    }

    let (solver, api_key) = svc
        .register(name, req.email.as_deref(), req.webhook_url.as_deref())
        .await
        .map_err(map_error)?;

    Ok((
        StatusCode::CREATED,
        Json(RegisterSolverResponse {
            solver_id: solver.id,
            api_key,
            name: solver.name,
        }),
    ))
}

// ── Profile (protected — solver owns this) ──────────────

pub async fn get_solver(
    State(svc): State<Arc<SolverReputationService>>,
    Path(id): Path<Uuid>,
) -> Result<Json<SolverPublic>, (StatusCode, String)> {
    svc.get_solver(id)
        .await
        .map(|s| Json(SolverPublic::from(s)))
        .map_err(map_error)
}

pub async fn update_solver(
    State(svc): State<Arc<SolverReputationService>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateSolverRequest>,
) -> Result<Json<SolverPublic>, (StatusCode, String)> {
    let solver = svc
        .update_profile(
            id,
            req.name.as_deref(),
            req.email.as_deref(),
            req.webhook_url.as_deref(),
        )
        .await
        .map_err(map_error)?;
    Ok(Json(SolverPublic::from(solver)))
}

pub async fn deactivate_solver(
    State(svc): State<Arc<SolverReputationService>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    svc.deactivate(id).await.map_err(map_error)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Dashboard ───────────────────────────────────────────

pub async fn get_dashboard(
    State(svc): State<Arc<SolverReputationService>>,
    Path(id): Path<Uuid>,
) -> Result<Json<SolverDashboard>, (StatusCode, String)> {
    svc.get_dashboard(id).await.map(Json).map_err(map_error)
}

// ── Leaderboard (public) ────────────────────────────────

pub async fn get_top_solvers(
    State(svc): State<Arc<SolverReputationService>>,
    Query(query): Query<TopSolversQuery>,
) -> Result<Json<Vec<SolverPublic>>, (StatusCode, String)> {
    let solvers = svc.get_top_solvers(query.limit).await.map_err(map_error)?;
    Ok(Json(solvers.into_iter().map(SolverPublic::from).collect()))
}

// ── Stats ───────────────────────────────────────────────

pub async fn get_solver_stats(
    State(svc): State<Arc<SolverReputationService>>,
    Path(id): Path<Uuid>,
) -> Result<Json<SolverStats>, (StatusCode, String)> {
    svc.get_solver_stats(id).await.map(Json).map_err(map_error)
}

pub async fn get_leaderboard(
    State(svc): State<Arc<SolverReputationService>>,
    Query(query): Query<TopSolversQuery>,
) -> Result<Json<Vec<LeaderboardEntry>>, (StatusCode, String)> {
    svc.get_leaderboard(query.limit)
        .await
        .map(Json)
        .map_err(map_error)
}

// ── Error mapping ───────────────────────────────────────

fn map_error(e: SolverError) -> (StatusCode, String) {
    match e {
        SolverError::NotFound => (StatusCode::NOT_FOUND, e.to_string()),
        SolverError::NameTaken => (StatusCode::CONFLICT, e.to_string()),
        SolverError::DbError(_) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}
