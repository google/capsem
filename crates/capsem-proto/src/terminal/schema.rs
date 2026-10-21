//! Schema budgets are generated from the same policy as runtime validation.
use crate::{TERMINAL_EVENTS_PER_UPDATE, TERMINAL_MAX_COLS, TERMINAL_MAX_ROWS, TERMINAL_METADATA_BYTES};
use utoipa::openapi::schema::{Array, ArrayBuilder, Object, ObjectBuilder, Type};
use utoipa::openapi::Ref;

pub fn text() -> Object {
    ObjectBuilder::new()
        .schema_type(Type::String)
        .max_length(Some(TERMINAL_METADATA_BYTES))
        .build()
}
fn dimension(maximum: usize) -> Object {
    ObjectBuilder::new()
        .schema_type(Type::Integer)
        .minimum(Some(1))
        .maximum(Some(maximum))
        .build()
}
pub fn cols() -> Object {
    dimension(TERMINAL_MAX_COLS)
}
pub fn rows() -> Object {
    dimension(TERMINAL_MAX_ROWS)
}
pub fn events() -> Array {
    ArrayBuilder::new()
        .items(Ref::from_schema_name("TerminalEvent"))
        .max_items(Some(TERMINAL_EVENTS_PER_UPDATE))
        .build()
}
