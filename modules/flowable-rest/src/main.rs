use flowable_platform_bootstrap::FlowablePlatform;
use flowable_rest::run_platform_server;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let platform = FlowablePlatform::bootstrap_from_sources(None)?;
    println!(
        "Starting Flowable REST Service with config: {:?}",
        platform.config()
    );

    let listener = TcpListener::bind(&platform.config().server.bind_address).await?;
    println!("Listening on {}", platform.config().server.bind_address);
    run_platform_server(platform, listener).await?;

    Ok(())
}
