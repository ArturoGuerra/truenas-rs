/* Handles the state of the socket, http connections etc, keeping track of requests and responses
* allowing for a seamless calling convention for methods */

use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot};

mod messaging;
mod protocol;

use messaging::*;

pub(crate) struct StateManager {
    // TODO: Create callback type.
    pending_calls: HashMap<RequestId, oneshot::Sender<()>>,

    // TODO: Create callback type and add channels to unbsubscribe.
    method_subscriptions: HashMap<MethodSubscription, mpsc::UnboundedSender<()>>,

    cmd_rx: mpsc::UnboundedReceiver<Cmd>,

    to_write_tx: mpsc::UnboundedSender<WireOut>,

    from_read_rx: mpsc::UnboundedReceiver<WireIn>,
}

impl StateManager {
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

    pub async fn task(&self) -> Result<(), Error> {
        Ok(())
    }
}
