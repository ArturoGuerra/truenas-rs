use bytes::Bytes;

pub mod http;
pub mod ws;

pub trait TransportSend {
    type Error: std::fmt::Debug + Send + 'static;

    fn send(&self, _bytes: Bytes) -> impl Future<Output = Result<(), Self::Error>> + Send + '_ {
        async { Ok(()) }
    }

    fn ping(&self, _bytes: Bytes) -> impl Future<Output = Result<(), Self::Error>> + Send + '_ {
        async { Ok(()) }
    }

    fn pong(&self, _bytes: Bytes) -> impl Future<Output = Result<(), Self::Error>> + Send + '_ {
        async { Ok(()) }
    }

    fn close(&self) -> impl Future<Output = Result<(), Self::Error>> + Send + '_ {
        async { Ok(()) }
    }
}

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

pub trait TransportRecv {
    type Error: std::fmt::Debug + Send + 'static;

    fn recv(&self) -> impl Future<Output = Result<Option<Event>, Self::Error>> + Send + '_ {
        async { Ok(None) }
    }
}
