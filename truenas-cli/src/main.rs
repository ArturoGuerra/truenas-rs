use futures_util::StreamExt;
use tokio_tungstenite::connect_async;
use truenas::{
    client::{Client, Response},
    types::ArrayParams,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + 'static>> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("provider already set elsewhere");

    let url = "wss://10.10.20.20/api/current";

    let (ws_stream, _) = connect_async(url).await.expect("failed to connect");

    let (write, read) = ws_stream.split();

    let client = Client::build_from_transport(write, read)
        .await
        .expect("failed to create client");

    println!("Client");

    let mut params = ArrayParams::default();
    params.insert(std::env::var("TRUENAS_API_KEY").unwrap())?;

    println!("Params");

    let r: Response<bool> = client
        .call("auth.login_with_api_key".into(), params)
        .await?;

    println!("Result: {:?}", r);

    Ok(())
}
