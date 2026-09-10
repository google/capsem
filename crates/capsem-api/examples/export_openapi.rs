fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", capsem_api::openapi().to_pretty_json()?);
    Ok(())
}
