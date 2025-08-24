// Used for internal messaging within the different threads needed for the websocket connection.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::ddp::protocol;

pub struct Document {
    id: String,
    fields: Option<Value>,
}

pub enum Event {}

pub enum Command {
    Sub {
        id: String,
        name: String,
        params: Option<Value>,
        ready: oneshot::Sender<()>,
    },
    UnSub {
        id: String,
    },
    Method {
        method: String,
        params: Option<Vec<Value>>,
        id: String,
        random_seed: Option<Value>,

        // Extra Fields
        result: oneshot::Sender<Result<Value, protocol::Error>>,
    },
    Ping {
        id: Option<String>,
    },
    Pong {
        id: Option<String>,
    },
}
