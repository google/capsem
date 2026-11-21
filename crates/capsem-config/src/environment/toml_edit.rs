//! Decode an edit against the verified source document without trusting its pins.
use super::*;
use toml::{Table, Value};

#[cfg(test)]
mod tests;

pub(super) fn document_tables(source: &str) -> Result<(EnvironmentIdentity, Table), String> {
    if source.len() > capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES {
        return Err("environment specification exceeds its byte limit".into());
    }
    let mut specification: Table = toml::from_str(source).map_err(|_| "invalid environment TOML")?;
    let mut identity = Table::new();
    for key in [
        "schema_version",
        "id",
        "revision_id",
        "parent_environment_id",
        "parent_revision_id",
    ] {
        if let Some(value) = specification.remove(key) {
            identity.insert(key.into(), value);
        }
    }
    let identity = Value::Table(identity)
        .try_into()
        .map_err(|_| "invalid environment identity")?;
    Ok((identity, specification))
}

impl EnvironmentDocument {
    /// Convert canonical TOML edits to typed intent. New selectors omit old
    /// resolution fields. Identical fields are checked against the observed
    /// base and stripped before the resolver makes any admission decision.
    pub fn draft_from_toml(&self, source: &str) -> Result<EnvironmentDraft, String> {
        let (identity, mut specification) = document_tables(source)?;
        if identity != self.identity {
            return Err("environment revision identity cannot be edited".into());
        }
        let original = &self.specification;
        if let Some(image) = table(&mut specification, "image") {
            let pin = original
                .image
                .as_ref()
                .filter(|pin| text(image, "reference") == Some(pin.reference.as_str()));
            strip_pin(image, "resolved_digest", pin.map(|pin| pin.resolved_digest.as_str()))?;
        }
        if let Some(repositories) = specification.get_mut("repositories").and_then(Value::as_array_mut) {
            for repository in repositories.iter_mut().filter_map(Value::as_table_mut) {
                let pin = original.repositories.iter().find(|pin| {
                    text(repository, "url") == Some(pin.url.as_str())
                        && text(repository, "ref") == Some(pin.reference.as_str())
                });
                strip_pin(
                    repository,
                    "resolved_commit",
                    pin.map(|pin| pin.resolved_commit.as_str()),
                )?;
            }
        }
        catalog(table(&mut specification, "skills"), &original.skills)?;
        for (section, original) in [
            ("access", &original.access.catalog),
            ("detection", &original.detection.catalog),
            ("mcps", &original.mcps.catalog),
        ] {
            catalog(
                table(&mut specification, section).and_then(|section| table(section, "catalog")),
                original,
            )?;
        }
        let draft: EnvironmentDraft = Value::Table(specification)
            .try_into()
            .map_err(|_| "invalid environment specification")?;
        draft.validate()?;
        Ok(draft)
    }
}

fn table<'a>(parent: &'a mut Table, key: &str) -> Option<&'a mut Table> {
    parent.get_mut(key).and_then(Value::as_table_mut)
}

fn text<'a>(table: &'a Table, key: &str) -> Option<&'a str> {
    table.get(key).and_then(Value::as_str)
}

fn strip_pin(table: &mut Table, key: &str, verified: Option<&str>) -> Result<(), String> {
    if let Some(field) = table.remove(key) {
        if verified.is_none() || field.as_str() != verified {
            return Err(format!(
                "{key} does not match the saved selector. Remove this field when changing a selector."
            ));
        }
    }
    Ok(())
}

fn catalog(
    input: Option<&mut Table>,
    original: &EnvironmentCatalogSelections<EnvironmentCatalogPin>,
) -> Result<(), String> {
    let Some(selected) = input
        .and_then(|input| input.get_mut("selected"))
        .and_then(Value::as_array_mut)
    else {
        return Ok(());
    };
    for entry in selected.iter_mut().filter_map(Value::as_table_mut) {
        let pin = original.selected.iter().find(|pin| {
            text(entry, "id") == Some(pin.id.as_str()) && text(entry, "version") == Some(pin.version.as_str())
        });
        strip_pin(
            entry,
            "catalog_reference",
            pin.map(|pin| pin.catalog_reference.as_str()),
        )?;
        strip_pin(entry, "resolved_digest", pin.map(|pin| pin.resolved_digest.as_str()))?;
    }
    Ok(())
}
