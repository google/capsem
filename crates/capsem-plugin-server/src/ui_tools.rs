use axum::{extract::State, Json};
use capsem_ui_catalog::ui_tools::{
    acceptance_program, run_tool_program, UiToolProgram, UiToolProgramResult,
};
use serde::Serialize;

use crate::AppState;

pub async fn run(
    State(state): State<AppState>,
    Json(program): Json<UiToolProgram>,
) -> Json<UiToolProgramResult> {
    let result = run_tool_program(program);
    *state.authored_ui.write().await = Some(result.clone());
    Json(result)
}

pub async fn acceptance() -> Json<UiToolProgramResult> {
    Json(run_tool_program(acceptance_program()))
}

pub async fn latest(State(state): State<AppState>) -> Json<LatestUiToolResult> {
    Json(LatestUiToolResult {
        result: state.authored_ui.read().await.clone(),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestUiToolResult {
    pub result: Option<UiToolProgramResult>,
}
