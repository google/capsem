//! Print the capsem.stream.v3 golden frames (sdk/specification/stream-v3.json).
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        serde_json::to_string_pretty(&capsem_api::stream::golden_frames())?
    );
    Ok(())
}
