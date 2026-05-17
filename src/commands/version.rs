// Implements REQ-0037 (tool version reporting).
use anyhow::Result;
use serde_json::json;

use crate::cli::VersionArgs;

pub fn run(args: VersionArgs) -> Result<()> {
    // Use the binary name ("req") so `req version` and clap's --version/-v/-V
    // agree exactly. The crates.io package name (CARGO_PKG_NAME = "req-cli")
    // is surfaced separately under the "package" field in JSON.
    let bin_name = "req";
    let version = env!("CARGO_PKG_VERSION");
    let package = env!("CARGO_PKG_NAME");
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "name": bin_name,
                "package": package,
                "version": version,
                "file_format": crate::storage::FORMAT_TAG,
                "mcp_protocol": "2024-11-05",
            }))?
        );
    } else {
        println!("{} {}", bin_name, version);
    }
    Ok(())
}
