use futures_util::StreamExt;
use tokio_tungstenite::connect_async;
use truenas::{
    client::{Client, Response},
    types::{ArrayParams, ObjectParams},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + 'static>> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("provider already set elsewhere");

    let url = std::env::var("TRUENAS_WS")?;

    let (ws_stream, _) = connect_async(url).await.expect("failed to connect");

    let (write, read) = ws_stream.split();

    let client = Client::build_from_transport(write, read)
        .await
        .expect("failed to create client");

    println!("Client");

    let mut params = ArrayParams::default();
    params.insert(std::env::var("TRUENAS_API_KEY")?)?;

    println!("Params");

    let r: Response<bool> = client
        .call("auth.login_with_api_key".into(), params)
        .await?;

    println!("Result: {:?}", r);

    let mut params = ArrayParams::default();
    params.insert("pool")?;
    params.insert("zebraTank")?;
    let r: Response<serde_json::Value> = client.call("pool.scrub.create".into(), params).await?;
    println!("Result: {:?}", r);

    Ok(())
}
