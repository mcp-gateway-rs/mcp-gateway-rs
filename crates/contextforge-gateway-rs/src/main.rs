mod logging;
mod runtime;

use std::sync::Arc;

use clap::Parser;
use contextforge_gateway_rs_lib::Config;

use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;

use tikv_jemallocator::Jemalloc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;
#[allow(clippy::print_stdout)]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = Config::parse();
    println!("contextforge-gateway-rs {config:?}");
    let _guard = logging::init_tracing_logging(&config);

    let local_session_manager = Arc::new(LocalSessionManager::default());

    let runtime = runtime::Runtime::from(&config);

    _ = runtime.execute(config, local_session_manager);

    Ok(())
}
