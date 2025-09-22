use crate::transport::{Capabilities, Close, Event, Transport};
use futures::task::{Context, Poll};
use futures_util::{Sink, Stream};
use std::fmt::Debug;
use std::pin::Pin;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{
        Error as TungsteniteError, Message, Utf8Bytes,
        protocol::{CloseFrame, frame::coding::CloseCode},
    },
};

#[derive(thiserror::Error, Debug)]
pub enum WsError {
    #[error("transport error: {0}")]
    Transport(#[from] TungsteniteError),
}

pub struct WsClient {
    ws_url: String,
}

pub struct WsConn {
    inner: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

impl WsConn {
    fn new(inner: WebSocketStream<MaybeTlsStream<TcpStream>>) -> Self {
        Self { inner }
    }
}

impl Sink<Event> for WsConn {
    type Error = WsError;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.inner)
            .poll_ready(cx)
            .map_err(WsError::Transport)
    }

    fn start_send(mut self: Pin<&mut Self>, item: Event) -> Result<(), Self::Error> {
        let msg = match item {
            Event::Data(b) => Message::Text(unsafe { Utf8Bytes::from_bytes_unchecked(b) }),
            Event::Ping(b) => Message::Ping(b),
            Event::Pong(b) => Message::Pong(b),
            Event::Close(cf) => Message::Close(cf.map(|c| CloseFrame {
                code: CloseCode::from(c.code as u16),
                reason: c.reason.into(),
            })),
        };
        Pin::new(&mut self.inner)
            .start_send(msg)
            .map_err(WsError::Transport)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.inner)
            .poll_flush(cx)
            .map_err(WsError::Transport)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.inner)
            .poll_close(cx)
            .map_err(WsError::Transport)
    }
}

impl Stream for WsConn {
    type Item = Result<Event, WsError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(Message::Binary(b)))) => Poll::Ready(Some(Ok(Event::Data(b)))),
            Poll::Ready(Some(Ok(Message::Text(t)))) => Poll::Ready(Some(Ok(Event::Data(t.into())))),
            Poll::Ready(Some(Ok(Message::Ping(b)))) => Poll::Ready(Some(Ok(Event::Ping(b.into())))),
            Poll::Ready(Some(Ok(Message::Pong(b)))) => Poll::Ready(Some(Ok(Event::Ping(b.into())))),
            Poll::Ready(Some(Ok(Message::Frame(_)))) => Poll::Ready(None),
            Poll::Ready(Some(Ok(Message::Close(c)))) => {
                Poll::Ready(Some(Ok(Event::Close(c.map(|c| Close {
                    code: u16::from(c.code) as i64,
                    reason: c.reason.to_string(),
                })))))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(WsError::Transport(e)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl WsClient {
    fn new(url: String) -> Self {
        Self { ws_url: url }
    }
}

impl Transport for WsClient {
    type Error = WsError;
    type TransportStream = WsConn;

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            reconnectable: true,
            bidirectional: true,
        }
    }

    fn connect(
        &mut self,
    ) -> impl Future<Output = Result<Self::TransportStream, Self::Error>> + Send + Sync + '_ {
        async move {
            let (ws, _) = connect_async(&self.ws_url).await?;
            Ok(WsConn::new(ws))
        }
    }

    fn reconnect(
        &mut self,
    ) -> impl Future<Output = Result<Self::TransportStream, Self::Error>> + Send + Sync + '_ {
        async {
            let (ws, _) = connect_async(&self.ws_url).await?;
            Ok(WsConn::new(ws))
        }
    }
}
