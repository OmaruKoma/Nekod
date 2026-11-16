use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use nekod_core::NekodAppService;
use nekod_mcp::NekodMcpServer;
use nekod_trainer_runtime::DeclarativeRuntime;

fn parse_data_dir() -> PathBuf {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--data-dir"
            && let Some(path) = args.next()
        {
            return PathBuf::from(path);
        }
    }
    env::temp_dir().join("nekod-mcp-dev")
}

fn main() {
    let data_dir = parse_data_dir();
    let runtime = Arc::new(DeclarativeRuntime::new());
    let service = match NekodAppService::open_in_dir(&data_dir, runtime) {
        Ok(svc) => Arc::new(svc),
        Err(err) => {
            eprintln!("nekod-mcp failed to initialize core service: {err}");
            std::process::exit(1);
        }
    };

    let server = NekodMcpServer::new(service);
    if let Err(err) = server.run_stdio() {
        eprintln!("nekod-mcp stdio error: {err}");
        std::process::exit(1);
    }
}
