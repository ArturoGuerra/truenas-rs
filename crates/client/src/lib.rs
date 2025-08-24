use futures::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use thiserror::Error;
use tokio::{
    net::TcpStream,
    sync::{mpsc, oneshot},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Error as TungsteniteError, Message, client::IntoClientRequest},
};
use uuid::Uuid;

mod ddp;

use ddp::{messaging, protocol};

#[derive(Error, Debug)]
pub enum Error {
    #[error("tungstenite error: {0}")]
    Tungstenite(TungsteniteError),
    #[error("channel send error: {0}")]
    SendError(tokio::sync::mpsc::error::SendError<messaging::Command>),
    #[error("oneshot channel recv error: {0}")]
    RecvError(tokio::sync::oneshot::error::RecvError),
    #[error("protocol error: {0}")]
    ProtocolError(ddp::protocol::Error),
    #[error("serde error: {0}")]
    SerdeJsonError(serde_json::Error),
}

async fn event_loop(
    mut cmd_rx: mpsc::UnboundedReceiver<messaging::Command>,
    ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
) {
    let (mut ws_write, mut ws_read) = ws_stream.split();

    let mut collections: HashMap<String, messaging::Document> = HashMap::new();
    let mut pending_pings: HashSet<String> = HashSet::new();
    let mut pending_subs: HashMap<String, oneshot::Sender<()>> = HashMap::new();
    let mut pending_methods: HashMap<String, oneshot::Sender<Result<Value, ddp::protocol::Error>>> =
        HashMap::new();

    loop {
        if let Some(msg) = ws_read.next().await {
            if let Ok(msg) = msg {
                let event: ddp::protocol::Event =
                    serde_json::from_str(msg.to_text().unwrap()).unwrap();
                match event {
                    protocol::Event::Connected { session } => {
                        println!("Connected session id: {}", session);
                    }
                    protocol::Event::Failed { version } => {
                        println!("Version: {}", &version);
                        break;
                    }
                    protocol::Event::NoSub { id, error } => {}
                    protocol::Event::Added {
                        collection,
                        id,
                        fields,
                    } => {}
                    protocol::Event::Changed {
                        collection,
                        id,
                        fields,
                        cleared,
                    } => {}
                    protocol::Event::Removed { collection, id } => {}
                    protocol::Event::Ready { subs } => {
                        for sub in subs {
                            if let Some(signal) = pending_subs.remove(&sub) {
                                signal.send(()).unwrap();
                            }
                        }
                    }
                    protocol::Event::AddedBefore {
                        collection,
                        id,
                        fields,
                        before,
                    } => {}
                    protocol::Event::MovedBefore {
                        collection,
                        id,
                        before,
                    } => {}
                    protocol::Event::Result { id, result, error } => {}
                    protocol::Event::Updated { methods } => {}
                    protocol::Event::Ping { id } => {
                        let cmd = protocol::Command::Ping { id };
                        ws_write
                            .send(Message::Text(serde_json::to_string(&cmd).unwrap().into()))
                            .await
                            .unwrap();
                    }
                    protocol::Event::Pong { id } => {
                        if let Some(id) = id
                            && !pending_pings.remove(&id)
                        {
                            break;
                        }
                    }
                }
            }
        }

        if let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                messaging::Command::Sub {
                    id,
                    name,
                    params,
                    ready,
                } => {
                    ws_write
                        .send(Message::Text(
                            protocol::Command::Sub {
                                id: id.clone(),
                                name,
                                params,
                            }
                            .into(),
                        ))
                        .await
                        .unwrap();
                    pending_subs.insert(id, ready);
                }
                messaging::Command::UnSub { id } => {
                    ws_write
                        .send(Message::Text(protocol::Command::UnSub { id }.into()))
                        .await
                        .unwrap();
                }
                messaging::Command::Method {
                    method,
                    params,
                    id,
                    random_seed,
                    result,
                } => {
                    ws_write
                        .send(Message::Text(
                            protocol::Command::Method {
                                method,
                                params,
                                id: id.clone(),
                                random_seed,
                            }
                            .into(),
                        ))
                        .await
                        .unwrap();
                    pending_methods.insert(id, result);
                }
                messaging::Command::Ping { id } => {
                    ws_write
                        .send(Message::Text(protocol::Command::Ping { id }.into()))
                        .await
                        .unwrap();
                }
                messaging::Command::Pong { id } => {
                    ws_write
                        .send(Message::Text(protocol::Command::Ping { id }.into()))
                        .await
                        .unwrap();
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct Client {
    cmd_tx: mpsc::UnboundedSender<messaging::Command>,
}

impl Client {
    async fn connect<R: IntoClientRequest + Unpin>(
        url: R,
        version: impl Into<String>,
    ) -> Result<Self, Error> {
        let (stream, _) = connect_async(url).await.map_err(Error::Tungstenite)?;

        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<messaging::Command>();

        Ok(Self { cmd_tx })
    }

    // TODO: Figure out interface for method calls.
    async fn call<R: DeserializeOwned>(
        &self,
        method: impl Into<String>,
        params: Option<Vec<Value>>,
    ) -> Result<R, Error> {
        let (tx, rx) = oneshot::channel::<Result<Value, protocol::Error>>();

        let id = Uuid::new_v4().to_string();

        let command = messaging::Command::Method {
            method: method.into(),
            params,
            id,
            random_seed: None,
            result: tx,
        };

        self.cmd_tx.send(command).map_err(Error::SendError)?;

        let res: Value = rx
            .await
            .map_err(Error::RecvError)?
            .map_err(Error::ProtocolError)?;

        serde_json::from_value(res).map_err(Error::SerdeJsonError)
    }

    //TODO: Figure out how subscriptions will be sent, channel of sorts will have to be returned
    //and we filter on our end or maybe allow for a callback trait to be passed?.
    async fn subscribe(&self) {}

    //TODO: Figure out how to unsubscribe, maybe this should be a method that can only be called
    //from a subscription trait.
    async fn unsubscribe(&self) {}

    // closes the connection to truenas destroying all queues.
    async fn close(self) {}
}
