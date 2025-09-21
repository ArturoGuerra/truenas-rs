use crate::error::Error;
use crate::io::{read_task, write_task};
use crate::state::State;
use crate::transport::{TransportRecv, TransportSend};
use crate::types::{
    Cmd, CmdRx, CmdTx, IntoParams, MethodIdBuf, RequestId, RequestIdBuf, Result, SubscriptionRecv,
    WireIn, WireOut,
};
use serde::de::DeserializeOwned;
use std::borrow::Borrow;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug)]
pub struct Response<T: DeserializeOwned> {
    pub id: RequestIdBuf,
    pub result: T,
}

impl<T> Response<T>
where
    T: DeserializeOwned,
{
    pub fn unwrap(self) -> T {
        self.result
    }

    pub fn id(&self) -> &RequestId {
        self.id.as_ref()
    }
}

impl<T> AsRef<RequestId> for Response<T>
where
    T: DeserializeOwned,
{
    fn as_ref(&self) -> &RequestId {
        self.id.as_ref()
    }
}

impl<T> AsRef<T> for Response<T>
where
    T: DeserializeOwned,
{
    fn as_ref(&self) -> &T {
        &self.result
    }
}

impl<T> Borrow<T> for Response<T>
where
    T: DeserializeOwned,
{
    fn borrow(&self) -> &T {
        &self.result
    }
}

#[derive(Debug)]
pub struct InnerClient {
    cmd_tx: CmdTx,
}

// TODO: Impl future
#[derive(Debug)]
pub struct Subscription<T: DeserializeOwned> {
    _marker: std::marker::PhantomData<T>,
    method: Option<MethodIdBuf>,
    cmd: CmdTx,
    recv: Option<SubscriptionRecv>,
}

impl<T> Drop for Subscription<T>
where
    T: DeserializeOwned,
{
    fn drop(&mut self) {
        self.recv.take();

        self.cmd
            .send(Cmd::Unsubscribe {
                method: self.method.take().unwrap(),
            })
            .unwrap();
    }
}

// Health of the client
#[derive(Debug, Clone)]
pub struct Health {
    pub attempts: u64,
    pub reconnects: u64,
    pub last_error: Option<String>,
}

// Current state of the client's connection.
#[derive(Debug, Clone)]
enum ConnState {
    Connected,
    Connecting,
    Disconnected,
    ShuttingDown,
    Failed,
}

#[derive(Debug, Clone)]
pub struct Client {
    cmd: CmdTx,
    cancel: CancellationToken,
    conn_state: watch::Receiver<ConnState>,
    health: watch::Receiver<Health>,
    sup_handle: Arc<Mutex<Option<JoinHandle<Result<()>>>>>,
    reconnect_backoff: u64,
    ping_interval: u64,
}

// the whole client should use an internal thread and loop model that will use channels.
impl Client {
    pub async fn build_from_transport<TS, TR>(ts: TS, tr: TR) -> Result<Self>
    where
        TS: TransportSend + Send + Sync + 'static,
        TR: TransportRecv + Send + Sync + 'static,
    {
        let cancel = CancellationToken::new();
        let (conn_state_tx, conn_state_rx) = watch::channel(ConnState::Disconnected);
        let (health_tx, health_rx) = watch::channel(Health {
            attempts: 0,
            reconnects: 0,
            last_error: None,
        });

        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Cmd>();

        let sup_handle = tokio::spawn(Self::supervisor(
            cmd_rx,
            conn_state_tx,
            health_tx,
            cancel.clone(),
            ts,
            tr,
        ));

        Ok(Self {
            cmd: cmd_tx,
            cancel,
            conn_state: conn_state_rx,
            health: health_rx,
            sup_handle: Arc::new(Mutex::new(Some(sup_handle))),
            reconnect_backoff: 60,
            ping_interval: 10,
        })
    }

    // TODO: Since TR and TS only have one method each then they can be turned into a stream and a
    // sink.
    async fn supervisor<TS, TR>(
        cmd: CmdRx,
        conn_state: watch::Sender<ConnState>,
        health: watch::Sender<Health>,
        cancel: CancellationToken,
        ts: TS,
        tr: TR,
    ) -> Result<()>
    where
        TS: TransportSend + Send + Sync + 'static,
        TR: TransportRecv + Send + Sync + 'static,
    {
        let (wirein_tx, wirein_rx) = mpsc::unbounded_channel::<WireIn>();
        let (wireout_tx, wireout_rx) = mpsc::unbounded_channel::<WireOut>();

        let state_cancel = cancel.clone();
        let mut state = State::new(cmd, wireout_tx, wirein_rx);
        let _state_handle = tokio::spawn(async move {
            state.task(state_cancel).await.unwrap();
        });

        let read_cancel = cancel.clone();
        let _read_handle = tokio::spawn(read_task(wirein_tx, tr, read_cancel));
        let write_cancel = cancel.clone();
        let _write_handle = tokio::spawn(write_task(wireout_rx, ts, write_cancel));

        Ok(())
    }

    pub async fn call<T, P>(&self, method: MethodIdBuf, params: P) -> Result<Response<T>>
    where
        T: DeserializeOwned,
        P: IntoParams,
    {
        let id = Uuid::new_v4();
        let (tx, rx) = oneshot::channel();
        let cmd = Cmd::Call {
            id: id.into(),
            method,
            params: params.into_params()?,
            reply: tx,
        };

        self.cmd
            .send(cmd)
            .map_err(|e| Error::TokioSend(e.to_string()))?;

        rx.await
            .map_err(Error::TokioOneshotRecv)?
            .and_then(|payload| {
                Ok(Response {
                    id: payload.id,
                    result: payload.result.deserialize_owned::<T>()?,
                })
            })
    }

    pub async fn notification<P>(&self, method: MethodIdBuf, params: P) -> Result<()>
    where
        P: IntoParams,
    {
        let cmd = Cmd::Notification {
            method,
            params: params.into_params()?,
        };

        self.cmd
            .send(cmd)
            .map_err(|e| Error::TokioSend(e.to_string()))
    }

    pub async fn subscribe<T>(&self, method: MethodIdBuf) -> Result<Subscription<T>>
    where
        T: DeserializeOwned,
    {
        let (tx, rx) = oneshot::channel();
        let cmd = Cmd::Subscribe {
            method: method.clone(),
            ready: tx,
        };

        self.cmd
            .send(cmd)
            .map_err(|e| Error::TokioSend(e.to_string()))?;

        Ok(Subscription {
            _marker: std::marker::PhantomData,
            method: Some(method),
            cmd: self.cmd.clone(),
            recv: Some(rx.await.map_err(Error::TokioOneshotRecv)?),
        })
    }
}
