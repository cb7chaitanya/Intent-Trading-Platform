use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use super::model::{Solver, TopSolversQuery};
use super::service::{SolverError, SolverReputationService};

pub async fn get_solver(
    State(svc): State<Arc<SolverReputationService>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Solver>, (StatusCode, String)> {
    svc.get_solver(id)
        .await
        .map(Json)
        .map_err(map_error)
}

pub async fn get_top_solvers(
    State(svc): State<Arc<SolverReputationService>>,
    Query(query): Query<TopSolversQuery>,
) -> Result<Json<Vec<Solver>>, (StatusCode, String)> {
    svc.get_top_solvers(query.limit)
        .await
        .map(Json)
        .map_err(map_error)
}

fn map_error(e: SolverError) -> (StatusCode, String) {
    match e {
        SolverError::NotFound => (StatusCode::NOT_FOUND, e.to_string()),
        SolverError::DbError(_) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}
