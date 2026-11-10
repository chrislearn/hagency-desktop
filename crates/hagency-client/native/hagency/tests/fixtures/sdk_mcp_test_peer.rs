//! Standalone offline SDK test transport, never the installed OwnerHost CLI.
//! Preserves library protocol coverage without restoring a product MCP command.
#[path = "../../src/mcp/stdio.rs"]
mod stdio;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let owned = match args.as_slice() {
        [command] if command == "mcp" => false,
        [command, flag] if command == "mcp" && flag == "--owned-task-profile" => true,
        _ => std::process::exit(2),
    };
    if let Err(error) = stdio::run_stdio(owned) {
        eprintln!("{error}");
        std::process::exit(error.exit_code());
    }
}
