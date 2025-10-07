/* Handles the state of the socket, http connections etc, keeping track of requests and responses
* allowing for a seamless calling convention for methods */

use crate::{
    protocol::{self, Response, ResponseAny},
    types::{
        Cmd, CmdRx, JsonRpcVer, JsonSlice, MethodIdBuf, RequestIdBuf, RpcReply, RpcResultPayload,
        SubscriptionPayload, SubscriptionSender, WireIn, WireInRx, WireOut, WireOutTx,
    },
};
use bytes::Bytes;
use serde_json::value::RawValue;
use std::{borrow::Cow, collections::HashMap};
use tokio::{select, sync::mpsc, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("channel send error")]
    Send,
    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug)]
pub enum Event {
    NormalOperations,
    Backpreassure,
}

#[derive(Debug)]
pub enum Ctrl {}

type StateCtrlTx = mpsc::Sender<Ctrl>;
type StateCtrlRx = mpsc::Receiver<Ctrl>;
type StateEventTx = mpsc::Sender<Event>;
type StateEventRx = mpsc::Receiver<Event>;

pub(crate) struct StateTask {
    pending_calls: HashMap<RequestIdBuf, RpcReply>,
    method_subscriptions: HashMap<MethodIdBuf, SubscriptionSender>,

    cancel: CancellationToken,
    cmd_rx: CmdRx,
    data_tx: WireOutTx,
    data_rx: WireInRx,
    event_tx: StateEventTx,
    ctrl_rx: StateCtrlRx,

    req_timeout: Duration,
}

impl StateTask {
    pub fn new(
        cancel: CancellationToken,
        cmd_rx: CmdRx,
        data_tx: WireOutTx,
        data_rx: WireInRx,
        event_tx: StateEventTx,
        ctrl_rx: StateCtrlRx,
        req_timeout: Duration,
    ) -> Self {
        Self {
            pending_calls: HashMap::new(),
            method_subscriptions: HashMap::new(),
            cancel,
            cmd_rx,
            data_tx,
            data_rx,
            event_tx,
            ctrl_rx,
            req_timeout,
        }
    }

    pub(crate) async fn run(&mut self) -> Result<(), Error> {
        println!("started state task");

        loop {
            select! {
                _ = self.cancel.cancelled() => break,
                Some(write) = self.cmd_rx.recv() => self.writer(write).await?,
                Some(read) = self.data_rx.recv() => self.reader(read).await?,

            }
        }

        Ok(())
    }

    async fn reader(&mut self, data: WireIn) -> Result<(), Error> {
        println!("Handling read!");

        match data {
            WireIn::Data(bytes) => {
                let root = std::str::from_utf8(&bytes).map_err(Error::Utf8)?;
                match serde_json::from_slice::<ResponseAny>(&bytes)
                    .map(Response::try_from)
                    .flatten()
                {
                    Ok(resp) => match resp {
                        Response::RpcResponse(resp) => {}
                        Response::Notification(notification) => {}
                        Response::RpcError(err) => {}
                    },
                    Err(err) => {
                        //TODO: This is a serde error so it should be logged but it shouldnt cause any
                        // other side effects.
                        println!("error parsing data: {:?}", err);
                    }
                }
            }
        }

        Ok(())
    }

    #[inline]
    async fn handle_read(&mut self, bytes: Bytes) -> Result<(), StateError> {
        println!("Handling read!");
        println!("Pending calls: {:?}", &self.pending_calls);

        let root = std::str::from_utf8(&bytes).map_err(StateError::Utf8)?;
        match serde_json::from_slice::<ResponseAny>(&bytes)
            .map(Response::try_from)
            .flatten()
        {
            Ok(resp) => match resp {
                Response::RpcResponse(resp) => match self.pending_calls.remove(resp.id.as_ref()) {
                    Some(sender) => sender
                        .send(Ok(RpcResultPayload {
                            id: resp.id.into_owned(),
                            result: JsonSlice::from_raw(bytes.clone(), root, resp.result),
                        }))
                        .map_err(|_| StateError::Send),
                    None => Ok(()),
                },
                Response::RpcError(err) => match self.pending_calls.remove(err.id.as_ref()) {
                    Some(sender) => sender
                        .send(Err(Error::Protocol {
                            id: err.id.into_owned(),
                            code: err.error.code,
                            message: err.error.message.to_string(),
                            data: err.error.data.map(|v: Cow<'_, RawValue>| v.into_owned()),
                        }))
                        .map_err(|_| StateError::Send),
                    None => Ok(()),
                },
                Response::Notification(notification) => match self
                    .method_subscriptions
                    .get_mut(notification.method.as_ref())
                {
                    Some(subscription) => {
                        let payload = SubscriptionPayload(
                            notification
                                .params
                                .map(|params| JsonSlice::from_raw(bytes.clone(), root, params)),
                        );
                        subscription
                            .send(payload)
                            .map(|_| ())
                            .map_err(|_| StateError::Send)
                    }
                    None => Ok(()),
                },
            },
            Err(err) | Ok(Err(err)) => {
                println!("error parsing data: {:?}", err);
                Ok(())
            }
        }
    }

    #[inline]
    async fn handle_cmd(&mut self, cmd: Cmd) -> Result<(), StateError> {
        println!("Handing cmd");
        match cmd {
            Cmd::Call {
                id,
                method,
                params,
                reply,
            } => {
                let params: Option<Box<RawValue>> = params
                    .map(|p| p.into_raw())
                    .transpose()
                    .map_err(StateError::Serde)?;

                let payload = protocol::RpcRequest {
                    jsonrpc: JsonRpcVer,
                    id: Cow::Borrowed(id.as_ref()),
                    method: Cow::Borrowed(method.as_ref()),
                    params: params.as_deref(),
                };

                let payload = serde_json::to_vec(&payload)
                    .map(Bytes::from)
                    .map_err(StateError::Serde)?;

                self.pending_calls.insert(id, reply);
                self.to_write_tx
                    .send(WireOut::Send(payload))
                    .map_err(|_| StateError::Send)
            }
            Cmd::Notification { method, params } => {
                let params = params
                    .map(|p| p.into_raw())
                    .transpose()
                    .map_err(StateError::Serde)?;
                let payload = protocol::Notification {
                    jsonrpc: JsonRpcVer,
                    method: Cow::Borrowed(method.as_ref()),
                    params: params.as_deref(),
                };

                let payload = serde_json::to_vec(&payload)
                    .map(Bytes::from)
                    .map_err(StateError::Serde)?;

                self.to_write_tx
                    .send(WireOut::Send(payload))
                    .map_err(|_| StateError::Send)
            }
            Cmd::Subscribe { method, ready } => {
                match self.method_subscriptions.get_mut(method.as_ref()) {
                    Some(subscription) => {
                        let subcriber = subscription.subscribe();
                        ready.send(subcriber).map_err(|_| StateError::Send)
                    }
                    None => Ok(()),
                }
            }
            Cmd::Unsubscribe { method } => {
                if let Some(subscription) = self.method_subscriptions.get(method.as_ref())
                    && subscription.receiver_count() == 0
                {
                    self.method_subscriptions.remove(method.as_ref());
                };
                Ok(())
            }
        }
    }

    pub(crate) async fn task_old(&mut self, cancel: CancellationToken) -> Result<(), StateError> {
        println!("started state task");
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                // main thread -> worker (here)
                Some(cmd) = self.cmd_rx.recv() => self.handle_cmd(cmd).await?,
                // read thread -> worker (here)
                Some(read) = self.from_read_rx.recv() => {
                    match read {
                        WireIn::Recv(bytes) => self.handle_read(bytes).await?,
                        WireIn::Ping => {},
                        WireIn::Pong => {},
                        WireIn::Closed => {

                        },
                    }
                }
            }
        }
    }
}
