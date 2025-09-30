use crate::error::Error;
use crate::io::{IO, io_task};
use crate::state::{State, StateEvent};
use crate::transport::Transport;
use crate::types::{
    Cmd, CmdRx, CmdTx, IntoParams, MethodIdBuf, RequestId, RequestIdBuf, Result, SubscriptionRecv,
    SubscriptionSender, WireIn, WireOut,
};
use futures::Stream;
use serde::de::DeserializeOwned;
use std::borrow::Borrow;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::{self, JoinHandle};
use tokio::time;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const CMD_CHANNEL_CAPACITY: usize = 128;
const WIRE_OUT_CAPACITY: usize = 256;
const WIRE_IN_CAPACITY: usize = 256;
const IO_EVENT_CAPACITY: usize = 8;
const IO_CTRL_CAPACITY: usize = 4;
const STATE_EVENT_CAPACITY: usize = 8;
const STATE_CTRL_CAPACITY: usize = 4;
const OUTQ_BACKPREASSURE_THRESHOLD: usize = 192;
const OUTQ_CAPACITY: usize = 256;

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

#[derive(Debug)]
enum SupervisorEvent {
    Shutdown,
}

// the whole client should use an internal thread and loop model that will use channels.
impl Client {
    pub async fn build_from_transport<T>(transport: T) -> Result<Self>
    where
        T: Transport + Send + Sync + 'static,
    {
        let cancel = CancellationToken::new();
        let (conn_state_tx, conn_state_rx) = watch::channel(ConnState::Disconnected);
        let (health_tx, health_rx) = watch::channel(Health {
            attempts: 0,
            reconnects: 0,
            last_error: None,
        });

        let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>(CMD_CHANNEL_CAPACITY);

        let sup_handle = tokio::spawn(Self::supervisor(
            cmd_rx,
            conn_state_tx,
            health_tx,
            cancel.clone(),
            transport,
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

    async fn supervisor<T>(
        cmd: CmdRx,
        conn_state: watch::Sender<ConnState>,
        health: watch::Sender<Health>,
        cancel: CancellationToken,
        mut transport: T,
    ) -> Result<()>
    where
        T: Transport + Send + 'static,
    {
        let stream = transport.connect().await.map_err(Error::transport_err)?;

        let (wirein_tx, wirein_rx) = mpsc::channel(WIRE_IN_CAPACITY);
        let (wireout_tx, wireout_rx) = mpsc::channel(WIRE_OUT_CAPACITY);

        let (state_ctrl_tx, state_ctrl_rx) = mpsc::channel(STATE_CTRL_CAPACITY);
        let (state_event_tx, state_event_rx) = mpsc::channel(STATE_EVENT_CAPACITY);

        let mut state = State::new(
            cmd,
            wireout_tx,
            wirein_rx,
            state_event_tx,
            state_ctrl_rx,
            cancel.clone(),
        );

        let state_handle = tokio::spawn(async move { state.run().await });

        let (io_ctrl_tx, io_ctrl_rx) = mpsc::channel(IO_CTRL_CAPACITY);
        let (io_event_tx, io_event_rx) = mpsc::channel(IO_EVENT_CAPACITY);

        let mut io = IO::new(
            wirein_tx,
            wireout_rx,
            io_event_tx,
            io_ctrl_rx,
            cancel.clone(),
            stream,
        );

        let io_handle = tokio::spawn(async move { io.run().await });

        let io_cancel = cancel.clone();

        let mut health_state = Health {
            attempts: 0,
            reconnects: 0,
            last_error: None,
        };

        //let _ = tokio::join!(io_handle, state_handle);
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
