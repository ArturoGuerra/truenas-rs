use crate::{error::Error, transport::TransportSend};
use bytes::Bytes;
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

impl<T, SE, E> TransportSend for T
where
    T: Sink<Message, Error = SE> + Unpin + Send,
    E: From<SE> + From<WsError> + Send + 'static,
{
    type Error = E;
}
