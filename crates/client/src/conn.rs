use crate::{Error, messaging, protocol};
use futures::{FutureExt, SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    net::TcpStream,
    sync::{
        mpsc::{self, UnboundedSender},
        oneshot,
    },
    task::JoinHandle,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};
use uuid::Uuid;

pub struct JobHandle<T>
where
    T: DeserializeOwned,
{
    job_id: String,
    rx: oneshot::Receiver<Result<Option<Value>, messaging::Error>>,
    _marker: std::marker::PhantomData<T>,
}

impl<T> JobHandle<T>
where
    T: DeserializeOwned,
{
    pub fn id(&self) -> &str {
        &self.job_id
    }
}

impl<T> Future for JobHandle<T>
where
    T: DeserializeOwned + Unpin,
{
    type Output = Result<Option<T>, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.rx.poll_unpin(cx) {
            Poll::Ready(res) => match res {
                Ok(res) => match res {
                    Ok(v) => Poll::Ready(
                        v.map(|v| serde_json::from_value::<T>(v).map_err(Error::SerdeJsonError))
                            .transpose(),
                    ),
                    Err(err) => Poll::Ready(Err(Error::MessagingError(err))),
                },
                Err(err) => Poll::Ready(Err(Error::RecvError(err))),
            },
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Debug)]
pub struct Connection {
    cmd_tx: mpsc::UnboundedSender<messaging::Command>,
    pub worker: JoinHandle<()>,
}

impl Connection {
    pub(crate) fn new(
        cmd_tx: mpsc::UnboundedSender<messaging::Command>,
        worker: JoinHandle<()>,
    ) -> Self {
        Self { cmd_tx, worker }
    }
    pub async fn job<R>(
        &self,
        method: impl Into<String>,
        params: Option<Vec<Value>>,
    ) -> Result<JobHandle<R>, Error>
    where
        R: DeserializeOwned + Unpin,
    {
        let (tx, rx) = oneshot::channel::<Result<Option<Value>, messaging::Error>>();

        let id = Uuid::new_v4().to_string();

        let command = messaging::Command::Method {
            method: method.into(),
            params,
            id: id.clone(),
            random_seed: None,
            result: Some(tx),
        };

        self.cmd_tx.send(command).map_err(Error::SendError)?;

        Ok(JobHandle {
            job_id: id,
            rx,
            _marker: std::marker::PhantomData,
        })
    }

    pub async fn call<R>(
        &self,
        method: impl Into<String>,
        params: Option<Vec<Value>>,
    ) -> Result<Option<R>, Error>
    where
        R: DeserializeOwned + Unpin,
    {
        let (tx, rx) = oneshot::channel::<Result<Option<Value>, messaging::Error>>();

        let id = Uuid::new_v4().to_string();

        let command = messaging::Command::Method {
            method: method.into(),
            params,
            id,
            random_seed: None,
            result: Some(tx),
        };

        self.cmd_tx.send(command).map_err(Error::SendError)?;

        Ok(rx
            .await
            .map_err(Error::RecvError)?
            .map_err(Error::MessagingError)?
            .map(|v| serde_json::from_value(v).unwrap()))
    }

    pub async fn subscribe(
        &self,
        name: String,
        params: Option<Vec<String>>,
    ) -> Result<String, Error> {
        let (tx, rx) = oneshot::channel::<Result<(), messaging::Error>>();

        let id = Uuid::new_v4().to_string();

        let command = messaging::Command::Sub {
            id: id.clone(),
            name,
            params,
            ready: tx,
        };

        self.cmd_tx.send(command).map_err(Error::SendError)?;

        rx.await
            .map_err(Error::RecvError)?
            .map_err(Error::MessagingError)
            .map(|_| id)
    }

    pub async fn unsubscribe(&self, id: String) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel::<Result<(), messaging::Error>>();

        let command = messaging::Command::UnSub { id, ready: tx };

        self.cmd_tx.send(command).map_err(Error::SendError)?;

        rx.await
            .map_err(Error::RecvError)?
            .map_err(Error::MessagingError)
    }

    pub(crate) async fn event_loop(
        mut cmd_rx: mpsc::UnboundedReceiver<messaging::Command>,
        ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    ) {
        let (mut ws_write, mut ws_read) = ws_stream.split();

        // Stores collections the server sends.
        let mut collections: HashMap<String, messaging::Collection> = HashMap::new();

        // Added, Changed, Removed, event emitter.
        let mut collection_subs: HashMap<String, UnboundedSender<messaging::Event>> =
            HashMap::new();

        // Pending ping events.
        let mut pending_pings: HashSet<String> = HashSet::new();

        // Pending sub events.
        let mut pending_subs: HashMap<String, oneshot::Sender<Result<(), messaging::Error>>> =
            HashMap::new();

        // Pending no sub events.
        let mut pending_nosubs: HashMap<String, oneshot::Sender<Result<(), messaging::Error>>> =
            HashMap::new();

        // pending method returns.
        let mut pending_methods: HashMap<
            String,
            oneshot::Sender<Result<Option<Value>, messaging::Error>>,
        > = HashMap::new();

        loop {
            tokio::select! {
                Some(msg) = ws_read.next() => {
                    match msg {
                        Ok(msg) => {
                            match serde_json::from_str(msg.to_text().unwrap()).unwrap() {
                                protocol::Event::Connected { session } => {
                                    println!("Connected session id: {}", session);
                                }
                                protocol::Event::Failed { version } => {
                                    println!("Version: {}", &version);
                                    break;
                                }
                                protocol::Event::NoSub { id, error } => {
                                    if let Some(nosub) = pending_nosubs.remove(&id) {
                                        let r = match error {
                                            Some(e) => Err(messaging::Error::ProtocolError(e)),
                                            None => Ok(()),
                                        };

                                        nosub.send(r).unwrap();
                                    }
                                }
                                protocol::Event::Added {
                                    collection,
                                    id,
                                    fields,
                                } => {
                                    if let Some(fields) = fields.clone() {
                                        let mut col = messaging::Collection::new();
                                        col.insert(id.clone(), fields);
                                        collections.insert(collection.clone(), col);
                                    }
                                    let document = messaging::Event::Added {
                                        collection: collection.clone(),
                                        id,
                                        fields,
                                    };

                                    if let Some(sub) = collection_subs.get_mut(&collection) {
                                        sub.send(document).unwrap();
                                    }
                                }
                                protocol::Event::Changed {
                                    collection,
                                    id,
                                    fields,
                                    cleared,
                                } => {
                                    if let Some(col) = collections.get_mut(&collection)
                                        && let Some(doc) = col.get_mut(&id)
                                    {
                                        if let Some(cleared) = cleared {
                                            for field in cleared {
                                                doc.remove(&field);
                                            }
                                        }

                                        if let Some(fields) = fields {
                                            for (key, field) in fields.into_iter() {
                                                match doc.get_mut(&key) {
                                                    Some(f) => {
                                                        *f = field;
                                                    }
                                                    None => {
                                                        doc.insert(key, field);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                protocol::Event::Removed { collection, id } => {
                                    if let Some(c) = collections.get_mut(&collection) {
                                        c.remove(&id);
                                        if c.is_empty() {
                                            collections.remove(&collection);
                                        }
                                    }
                                }
                                protocol::Event::Ready { subs } => {
                                    for sub in subs {
                                        if let Some(signal) = pending_subs.remove(&sub) {
                                            signal.send(Ok(())).unwrap();
                                        }
                                    }
                                }
                                protocol::Event::Result { id, result, error } => {
                                    if let Some(tx) = pending_methods.remove(&id) {
                                        if let Some(error) = error {
                                            tx.send(Err(messaging::Error::ProtocolError(error)))
                                                .unwrap();
                                        } else {
                                            tx.send(Ok(result)).unwrap();
                                        }
                                    }
                                }
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
                                _ => {}
                            }
                        }
                        Err(err) => {
                            println!("failed to deserialize message: {err}");
                            break;
                        }

                    }
                }

                Some(cmd) = cmd_rx.recv() => {
                    match cmd {
                        messaging::Command::Collection { collection, tx } => {
                            match collections.get(&collection) {
                                Some(col) => tx.send(Some(col.clone())).unwrap(),
                                None => tx.send(None).unwrap(),
                            };
                        }
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
                        messaging::Command::UnSub { id, ready } => {
                            ws_write
                                .send(Message::Text(
                                    protocol::Command::UnSub { id: id.clone() }.into(),
                                ))
                                .await
                                .unwrap();
                            pending_nosubs.insert(id, ready);
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
                            if let Some(tx) = result {
                                pending_methods.insert(id.clone(), tx);
                            }

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
    }
}
