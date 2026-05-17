// Implements REQ-0037 (tool version reporting).
use anyhow::Result;
use serde_json::json;

use crate::cli::VersionArgs;

pub fn run(args: VersionArgs) -> Result<()> {
    let name = env!("CARGO_PKG_NAME");
    let version = env!("CARGO_PKG_VERSION");
    if args.json {
        println!("{}", serde_json::to_string_pretty(&json!({
            "name": name,
            "version": version,
            "file_format": crate::storage::FORMAT_TAG,
            "mcp_protocol": "2024-11-05",
        }))?);
    } else {
        println!("{} {}", name, version);
    }
    Ok(())
}
