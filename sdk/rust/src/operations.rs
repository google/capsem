//! Typed HTTP operations generated from the gateway OpenAPI contract.

mod generated;
pub use generated::*;

fn enum_value(value: &impl serde::Serialize) -> crate::Result<String> {
    Ok(serde_json::from_str(&serde_json::to_string(value)?)?)
}

fn enum_values(values: &[impl serde::Serialize]) -> crate::Result<String> {
    Ok(values
        .iter()
        .map(enum_value)
        .collect::<crate::Result<Vec<_>>>()?
        .join(","))
}

#[cfg(test)]
mod tests;
