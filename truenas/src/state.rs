/* Handles the state of the socket, http connections etc, keeping track of requests and responses
* allowing for a seamless calling convention for methods */

use bytes::Bytes;
use serde_json::value::RawValue;
use std::borrow::Cow;
use std::collections::HashMap;

use crate::error::Error;
use crate::protocol::{self, Response, ResponseAny, RpcResponse};
use crate::types::{
    Cmd, CmdRx, JsonRpcVer, JsonSlice, MethodIdBuf, RequestIdBuf, RpcReply, RpcResultPayload,
    SubscriptionPayload, SubscriptionSender, WireIn, WireInRx, WireOut, WireOutTx,
};

pub(crate) struct State {
    pending_calls: HashMap<RequestIdBuf, RpcReply>,

    method_subscriptions: HashMap<MethodIdBuf, SubscriptionSender>,

    cmd_rx: CmdRx,

    to_write_tx: WireOutTx,

    from_read_rx: WireInRx,
}

#[derive(thiserror::Error, Debug)]
pub enum TaskError {
    #[error("channel send error")]
    Send,
    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

impl State {
    pub fn new(cmd_rx: CmdRx, to_write_tx: WireOutTx, from_read_rx: WireInRx) -> Self {
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
        println!("Handling read!");
        println!("Pending calls: {:?}", &self.pending_calls);

        let root = std::str::from_utf8(&bytes).map_err(TaskError::Utf8)?;
        match serde_json::from_slice::<ResponseAny>(&bytes).map(Response::try_from) {
            Ok(Ok(resp)) => match resp {
                Response::RpcResponse(resp) => match self.pending_calls.remove(resp.id.as_ref()) {
                    Some(sender) => sender
                        .send(Ok(RpcResultPayload {
                            id: resp.id.into_owned(),
                            result: JsonSlice::from_raw(bytes.clone(), root, resp.result),
                        }))
                        .map_err(|_| TaskError::Send),
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
                        .map_err(|_| TaskError::Send),
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
                            .map_err(|_| TaskError::Send)
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
    async fn handle_cmd(&mut self, cmd: Cmd) -> Result<(), TaskError> {
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
                    .map_err(TaskError::Serde)?;

                let payload = protocol::RpcRequest {
                    jsonrpc: JsonRpcVer,
                    id: Cow::Borrowed(id.as_ref()),
                    method: Cow::Borrowed(method.as_ref()),
                    params: params.as_deref(),
                };

                let payload = serde_json::to_vec(&payload)
                    .map(Bytes::from)
                    .map_err(TaskError::Serde)?;

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
                    jsonrpc: JsonRpcVer,
                    method: Cow::Borrowed(method.as_ref()),
                    params: params.as_deref(),
                };

                let payload = serde_json::to_vec(&payload)
                    .map(Bytes::from)
                    .map_err(TaskError::Serde)?;

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
        }
    }

    pub(crate) async fn task(&mut self) -> Result<(), TaskError> {
        println!("started state task");
        loop {
            tokio::select! {
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
