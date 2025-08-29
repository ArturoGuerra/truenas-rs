use crate::{Error, conn::Connection, messaging};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};

#[derive(Clone, Debug)]
pub struct Client<R: IntoClientRequest + Unpin + Clone> {
    url: R,
}

impl<R> Client<R>
where
    R: IntoClientRequest + Unpin + Clone,
{
    pub fn new(url: R) -> Self {
        Self { url }
    }

    pub async fn connect<F>(&self) -> Result<Connection, Error>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let (stream, _) = connect_async(self.url.clone())
            .await
            .map_err(Error::Tungstenite)?;

        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<messaging::Command>();

        let worker = tokio::spawn(async move {
            Connection::event_loop(cmd_rx, stream).await;
        });

        Ok(Connection::new(cmd_tx, worker))
    }
}
