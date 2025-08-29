// Used for internal messaging within the different threads needed for the websocket connection.

use crate::protocol;
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;
use tokio::sync::oneshot;

#[derive(Error, Debug)]
pub enum Error {
    #[error("protocol error: {0}")]
    ProtocolError(protocol::Error),
}

pub type Collection = HashMap<String, protocol::Fields>;

pub struct Document {
    pub collection: String,
    pub fields: Option<protocol::Fields>,
}

pub enum Event {
    Added {
        collection: String,
        id: String,
        fields: Option<protocol::Fields>,
    },
    Changed {
        collection: String,
        id: String,
        fields: Option<protocol::Fields>,
        cleared: Option<Vec<String>>,
    },
    Removed {
        collection: String,
        id: String,
    },
}

pub enum Command {
    Collection {
        collection: String,
        tx: oneshot::Sender<Option<Collection>>,
    },
    Sub {
        id: String,
        name: String,
        params: Option<Vec<String>>,
        ready: oneshot::Sender<Result<(), Error>>,
    },
    UnSub {
        id: String,
        ready: oneshot::Sender<Result<(), Error>>,
    },
    Method {
        method: String,
        params: Option<Vec<Value>>,
        id: String,
        random_seed: Option<Value>,

        // Extra Fields
        result: Option<oneshot::Sender<Result<Option<Value>, Error>>>,
    },
    Ping {
        id: Option<String>,
    },
    Pong {
        id: Option<String>,
    },
}
