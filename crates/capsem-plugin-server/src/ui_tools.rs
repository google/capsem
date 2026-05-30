use axum::Json;
use capsem_plugin_engine::ui_tools::{
    acceptance_program, run_tool_program, UiToolProgram, UiToolProgramResult,
};

pub async fn run(Json(program): Json<UiToolProgram>) -> Json<UiToolProgramResult> {
    Json(run_tool_program(program))
}

pub async fn acceptance() -> Json<UiToolProgramResult> {
    Json(run_tool_program(acceptance_program()))
}
