//! Dumps builtin MCP tool definitions to JSON on stdout.
//!
//! Used by `_generate-settings` to produce `config/mcp-tools.json`,
//! which the Python mock generator reads to create frontend mock data.

fn main() {
    let mut tools = capsem_core::mcp::builtin_tools::builtin_tool_defs();
    tools.extend(capsem_core::mcp::file_tools::file_tool_defs());
    println!("{}", serde_json::to_string_pretty(&tools).unwrap());
}
