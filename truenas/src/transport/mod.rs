use bytes::Bytes;

pub mod http;
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

pub trait TransportSend {
    type Error: std::fmt::Debug + Send + 'static;

    fn send(&mut self, _event: Event) -> impl Future<Output = Result<(), Self::Error>> + Send + '_ {
        async { Ok(()) }
    }
}

pub trait TransportRecv {
    type Error: std::fmt::Debug + Send + 'static;

    fn recv(&mut self) -> impl Future<Output = Result<Option<Event>, Self::Error>> + Send + '_ {
        async { Ok(None) }
    }
}
