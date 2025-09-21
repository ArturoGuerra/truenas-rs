use bytes::Bytes;
use futures::{Sink, Stream};
use std::error::Error;
use std::fmt::Debug;

pub mod ws;

#[derive(Debug, Clone)]
pub enum Event {
    Data(Bytes),
    Ping(Bytes),
    Pong(Bytes),
    Close(Option<Close>),
}

#[derive(Debug, Clone)]
pub struct Close {
    pub code: i64,
    pub reason: String,
}

pub struct Capabilities {
    pub reconnectable: bool,
    pub bidirectional: bool,
}

pub trait Transport {
    type Error: Error + Debug + Send + Sync + 'static;
    type TransportStream: Stream<Item = Result<Event, Self::Error>>
        + Sink<Event, Error = Self::Error>
        + Send
        + Sync
        + 'static;

    fn capabilities(&self) -> Capabilities;
    fn connect(
        &mut self,
    ) -> impl Future<Output = Result<Self::TransportStream, Self::Error>> + Send + Sync + 'static;
    fn reconnect(
        &mut self,
    ) -> impl Future<Output = Result<Self::TransportStream, Self::Error>> + Send + Sync + 'static;
}
