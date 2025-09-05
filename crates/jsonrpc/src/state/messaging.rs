use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

pub type RequestId = String;
pub type MethodSubscription = String;

// TODO: Create an actual error value for this.
pub type RpcError = String;
pub type Error = String;

pub(crate) enum WireIn {
    Recv(Value),
    Closed,
}

pub(crate) enum WireOut {
    Send(Value),
    Closed,
}

pub(crate) enum Cmd {
    Call {
        method: String,
        // TODO: Need to change this.
        params: Value,
        // TODO: Need to add return type.
        reply: oneshot::Sender<Result<(), RpcError>>,
    },

    Notification {
        method: String,
        params: Value,
    },

    Subscribe {
        method: String,
        // TODO: Need to change this to a Result with nothing and Error
        ready: oneshot::Sender<()>,
    },

    Unsubscribe {
        id: String,
    },
}

pub(crate) enum Event {
    Response { id: String, result: Value },
}
