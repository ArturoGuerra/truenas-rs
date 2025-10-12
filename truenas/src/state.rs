/* Handles the state of the socket, http connections etc, keeping track of requests and responses
* allowing for a seamless calling convention for methods */

use crate::{
    protocol::{self, Response, ResponseAny},
    types::{
        Cmd, CmdRx, JsonRpcVer, JsonSlice, MethodId, MethodIdBuf, RequestId, RpcError, RpcPayload,
        RpcReply, SubscriptionPayload, SubscriptionSender, WireIn, WireInRx, WireOut, WireOutTx,
    },
};
use bytes::Bytes;
use futures::{Stream, StreamExt};
use serde_json::value::RawValue;
use slab::Slab;
use std::{borrow::Cow, collections::HashMap, sync::Arc};
use tokio::{select, sync::mpsc, time::Duration};
use tokio_util::{
    sync::CancellationToken,
    time::{DelayQueue, delay_queue},
};

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

#[derive(Debug)]
struct PendingCalls {
    pub map: HashMap<RequestId, (delay_queue::Key, RpcReply)>,
    pub timers: DelayQueue<RequestId>,
}

impl Default for PendingCalls {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingCalls {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            timers: DelayQueue::new(),
        }
    }

    fn insert(&mut self, id: RequestId, callback: RpcReply, timeout: Duration) {
        if !self.map.contains_key(&id) {
            let timer_id = self.timers.insert(id, timeout);
            self.map.insert(id, (timer_id, callback));
        }
    }

    fn remove(&mut self, id: &RequestId) -> Option<RpcReply> {
        match self.map.remove(id) {
            Some((key, rpcreply)) => {
                self.timers.remove(&key);
                Some(rpcreply)
            }
            None => None,
        }
    }
}

pub(crate) struct StateTask {
    pending_calls: PendingCalls,
    method_subscriptions: HashMap<MethodIdBuf, SubscriptionSender>,

    cancel: CancellationToken,
    cmd_rx: CmdRx,
    data_tx: WireOutTx,
    data_rx: WireInRx,
    event_tx: StateEventTx,
    ctrl_rx: StateCtrlRx,
}

impl StateTask {
    pub fn new(
        cancel: CancellationToken,
        cmd_rx: CmdRx,
        data_tx: WireOutTx,
        data_rx: WireInRx,
        event_tx: StateEventTx,
        ctrl_rx: StateCtrlRx,
    ) -> Self {
        Self {
            pending_calls: PendingCalls::default(),
            method_subscriptions: HashMap::new(),
            cancel,
            cmd_rx,
            data_tx,
            data_rx,
            event_tx,
            ctrl_rx,
        }
    }

    pub(crate) async fn run(&mut self) -> Result<(), Error> {
        println!("started state task");

        loop {
            select! {
                _ = self.cancel.cancelled() => break,
                Some(read) = self.data_rx.recv() => self.reader(read).await?,
                Some(expired) = self.pending_calls.timers.next() => {
                    self.pending_calls.remove(&expired.into_inner());
                }

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
                        Response::RpcResponse(resp) => {
                            match self.pending_calls.remove(&resp.id) {
                                Some(sender) => {
                                    if let Err(_) = sender.send(Ok(RpcPayload {
                                        id: resp.id,
                                        result: JsonSlice::from_raw(
                                            bytes.clone(),
                                            root,
                                            resp.result,
                                        ),
                                    })) {
                                        //TODO: Log dropped callback channel.
                                    }
                                }
                                None => {
                                    //TODO: Log invalid request.
                                }
                            }
                        }
                        Response::Notification(notification) => {}
                        Response::RpcError(err) => {
                            match self.pending_calls.remove(&err.id) {
                                Some(sender) => {
                                    if let Err(_) = sender.send(Err(RpcError {
                                        id: err.id,
                                        code: err.error.code,
                                        message: err.error.message.to_string(),
                                        data: None,
                                    })) {
                                        //TODO: Log dropped sender.
                                    }
                                }
                                None => {
                                    //TODO: Err has an invalid id or no id.
                                }
                            }
                        }
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
}
