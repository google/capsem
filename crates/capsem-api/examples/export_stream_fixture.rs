//! Print the capsem.stream.v2 golden frames (sdk/specification/stream-v2.json).
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        serde_json::to_string_pretty(&capsem_api::stream::golden_frames())?
    );
    Ok(())
}
