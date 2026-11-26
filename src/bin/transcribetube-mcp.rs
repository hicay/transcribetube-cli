//! MCP server entry point (stdio). Same server as `tube mcp`.
use transcribetube_cli::{api::Client, config, mcp};

fn main() {
    let client = Client::new(config::load(), "mcp");
    mcp::run_stdio(client);
}
