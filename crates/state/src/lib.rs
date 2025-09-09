/* Handles the state of the socket, http connections etc, keeping track of requests and responses
* allowing for a seamless calling convention for methods */

use crate::core::{WireIn, WireOut};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::borrow::Cow;
use std::collections::HashMap;
use tokio::sync::{broadcast, mpsc, oneshot};

pub mod protocol;
pub mod types;

const JSONRPC_VERSION: &str = "2.0";

pub(crate) use types::{Cmd, RpcResultPayload};
pub use types::{
    Error, IntoParams, JsonSlice, MethodId, MethodIdBuf, ParamConvError, Params, RequestId,
    RpcReply,
};

use protocol::Response;

use crate::types::{RequestIdBuf, SubscriptionPayload, SubscriptionSender};

pub(crate) struct State {
    pending_calls: HashMap<RequestIdBuf, RpcReply>,

    method_subscriptions: HashMap<MethodIdBuf, SubscriptionSender>,

    cmd_rx: mpsc::UnboundedReceiver<Cmd>,

    to_write_tx: mpsc::UnboundedSender<WireOut>,

    from_read_rx: mpsc::UnboundedReceiver<WireIn>,
}

#[derive(thiserror::Error, Debug)]
pub enum TaskError {
    #[error("channel send error")]
    Send,
    #[error("utf8: {0}")]
    UTF8(std::str::Utf8Error),
    #[error("serde: {0}")]
    Serde(serde_json::Error),
}

impl State {
    pub fn new(
        cmd_rx: mpsc::UnboundedReceiver<Cmd>,
        to_write_tx: mpsc::UnboundedSender<WireOut>,
        from_read_rx: mpsc::UnboundedReceiver<WireIn>,
    ) -> Self {
        Self {
            pending_calls: HashMap::new(),
            method_subscriptions: HashMap::new(),
            cmd_rx,
            to_write_tx,
            from_read_rx,
        }
    }

    #[inline]
    async fn handle_read(&mut self, bytes: Bytes) -> Result<(), TaskError> {
        let root = std::str::from_utf8(&bytes).map_err(TaskError::UTF8)?;
        match serde_json::from_slice::<Response>(&bytes) {
            Ok(resp) => match resp {
                Response::RpcResponse(resp) => match self.pending_calls.remove(resp.id.as_ref()) {
                    Some(sender) => sender
                        .send(Ok(RpcResultPayload {
                            id: resp.id.into_owned(),
                            result: JsonSlice::from_raw(bytes.clone(), root, resp.result),
                        }))
                        .map_err(|_| TaskError::Send),
                    None => Ok(()),
                },
                Response::RpcError(err) => match err.id {
                    Some(id) => match self.pending_calls.remove(id.as_ref()) {
                        Some(sender) => sender
                            .send(Err(Error::Protocol {
                                id: id.into_owned(),
                                code: err.error.code,
                                message: err.error.message.to_string(),
                                data: err.error.data.map(|v: Cow<'_, RawValue>| v.into_owned()),
                            }))
                            .map_err(|_| TaskError::Send),
                        None => Ok(()),
                    },
                    None => {
                        println!("notification error: {:?}", &err);
                        Ok(())
                    }
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
                            .map_err(|_| TaskError::Send)
                    }
                    None => Ok(()),
                },
            },
            Err(err) => {
                println!("error parsing data: {:?}", err);
                Ok(())
            }
        }
    }

    #[inline]
    async fn handle_cmd(&mut self, cmd: Cmd) -> Result<(), TaskError> {
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
                    .map_err(TaskError::Serde)?;

                let payload = protocol::RpcRequest {
                    jsonrpc: JSONRPC_VERSION,
                    id: Cow::Borrowed(id.as_ref()),
                    method: Cow::Borrowed(method.as_ref()),
                    params: params.as_deref(),
                };

                let payload = serde_json::to_vec(&payload).map(Bytes::from).unwrap();

                self.pending_calls.insert(id, reply);
                self.to_write_tx
                    .send(WireOut::Send(payload))
                    .map_err(|_| TaskError::Send)
            }
            Cmd::Notification { method, params } => {
                let params = params
                    .map(|p| p.into_raw())
                    .transpose()
                    .map_err(TaskError::Serde)?;
                let payload = protocol::Notification {
                    jsonrpc: JSONRPC_VERSION,
                    method: Cow::Borrowed(method.as_ref()),
                    params: params.as_deref(),
                };

                let payload = serde_json::to_vec(&payload).map(Bytes::from).unwrap();

                self.to_write_tx
                    .send(WireOut::Send(payload))
                    .map_err(|_| TaskError::Send)
            }
            Cmd::Subscribe { method, ready } => {
                match self.method_subscriptions.get_mut(method.as_ref()) {
                    Some(subscription) => {
                        let subcriber = subscription.subscribe();
                        ready.send(subcriber).map_err(|_| TaskError::Send)
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
            Cmd::Close => Ok(()),
        }
    }

    pub(crate) async fn task(&mut self) -> Result<(), TaskError> {
        loop {
            tokio::select! {
                // main thread -> worker (here)
                Some(cmd) = self.cmd_rx.recv() => self.handle_cmd(cmd).await?,
                // read thread -> worker (here)
                Some(read) = self.from_read_rx.recv() => {
                    match read {
                        WireIn::Recv(bytes) => self.handle_read(bytes).await?,
                        WireIn::Closed => {

                        },
                    }
                }
            }
        }
    }
}
