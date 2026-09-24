use capsem_ui_catalog::editable_state::run_editable_state_audit;
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let audit = run_editable_state_audit()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": audit.schema,
            "baseSnapshotBytes": audit.base_snapshot_bytes,
            "updateBytes": audit.update_bytes,
            "updatedSnapshotBytes": audit.updated_snapshot_bytes,
            "updateBlake3": audit.update_blake3,
            "affectedIds": audit.record.payload.affected_ids,
            "topologyIds": audit.topology_ids,
        }))?
    );
    Ok(())
}
