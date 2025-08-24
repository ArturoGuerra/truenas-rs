// Structs and Enums used at ddp protocol level so these will be serialized and sent over the
// websocket.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use tokio_tungstenite::tungstenite::Utf8Bytes;
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Error {
    error: String,
    reason: Option<String>,
    message: Option<String>,
    #[serde(rename = "errorType")]
    error_type: Value,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "error: {} reason: {}, message: {}",
            self.error,
            self.reason.clone().unwrap_or("Unkown".into()),
            self.message.clone().unwrap_or("Unkown".into()),
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Event {
    Connected {
        session: String,
    },
    Failed {
        version: String,
    },
    NoSub {
        id: String,
        error: Option<Error>,
    },
    Added {
        collection: String,
        id: String,
        fields: Option<Value>,
    },
    Changed {
        collection: String,
        id: String,
        fields: Option<Value>,
        cleared: Vec<String>,
    },
    Removed {
        collection: String,
        id: String,
    },
    Ready {
        subs: Vec<String>,
    },
    AddedBefore {
        collection: String,
        id: String,
        fields: Option<Value>,
        before: Option<String>,
    },
    MovedBefore {
        collection: String,
        id: String,
        before: Option<String>,
    },
    Result {
        id: String,
        result: Option<Value>,
        error: Option<Error>,
    },
    Updated {
        methods: Vec<String>,
    },
    Ping {
        id: Option<String>,
    },
    Pong {
        id: Option<String>,
    },
}

impl From<Event> for String {
    fn from(e: Event) -> Self {
        serde_json::to_string(&e).unwrap()
    }
}

impl From<Event> for Utf8Bytes {
    fn from(e: Event) -> Self {
        serde_json::to_string(&e).unwrap().into()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Command {
    Sub {
        id: String,
        name: String,
        params: Option<Value>,
    },
    UnSub {
        id: String,
    },
    Method {
        method: String,
        params: Option<Vec<Value>>,
        id: String,
        #[serde(rename = "randomSeed")]
        random_seed: Option<Value>,
    },
    Ping {
        id: Option<String>,
    },
    Pong {
        id: Option<String>,
    },
}

impl From<Command> for String {
    fn from(c: Command) -> Self {
        serde_json::to_string(&c).unwrap()
    }
}

impl From<Command> for Utf8Bytes {
    fn from(c: Command) -> Self {
        serde_json::to_string(&c).unwrap().into()
    }
}
