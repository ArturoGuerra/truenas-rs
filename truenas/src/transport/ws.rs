use crate::transport::{Capabilities, Close, Event, Transport};
use futures::task::{Context, Poll};
use futures_util::{Sink, Stream, StreamExt};
use std::fmt::Debug;
use std::pin::Pin;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::{
    WebSocketStream, connect_async,
    tungstenite::{
        Error as TungsteniteError, Message,
        protocol::{CloseFrame, frame::Frame},
    },
};

#[derive(thiserror::Error, Debug)]
pub enum WsError {
    #[error("transport error: {0}")]
    Transport(#[from] TungsteniteError),
}

pub struct WsClient<S> {
    ws_url: String,
    _marker: std::marker::PhantomData<S>,
}

pub struct WsConn<S> {
    inner: WebSocketStream<S>,
}

impl<S> WsConn<S> {
    fn new(inner: WebSocketStream<S>) -> Self {
        Self { inner }
    }
}

impl<S> Sink<Event> for WsConn<S>
where
    WebSocketStream<S>: Sink<Message, Error = TungsteniteError> + Unpin,
{
    type Error = WsError;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.inner)
            .poll_ready(cx)
            .map_err(WsError::Transport)
    }

    fn start_send(mut self: Pin<&mut Self>, item: Event) -> Result<(), Self::Error> {
        let msg = match item {
            Event::Data(b) => Message::Binary(b.to_vec()), // one copy (WS needs Vec<u8>)
            Event::Ping(b) => Message::Ping(b.to_vec()),
            Event::Pong(b) => Message::Pong(b.to_vec()),
            Event::Close(cf) => Message::Close(cf.map(|c| CloseFrame {
                code: i16::from(c.code) as i64,
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

impl<S> Stream for WsConn<S>
where
    WebSocketStream<S>: Stream<Item = Result<Message, TungsteniteError>> + Unpin,
{
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
                    code: i16::from(c.code) as i64,
                    reason: c.reason.into(),
                })))))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(WsError::Transport(e)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S> WsClient<S> {
    fn new(url: String) -> Self {
        Self {
            ws_url: url,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<S> Transport for WsClient<S>
where
    S: AsyncRead + AsyncWrite + Sync + Unpin + Send + 'static,
{
    type Error = WsError;
    type TransportStream = WsConn<S>;

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            reconnectable: true,
            bidirectional: true,
        }
    }

    fn connect(
        &mut self,
    ) -> impl Future<Output = Result<Self::TransportStream, Self::Error>> + Send + Sync + 'static
    {
        async move {
            let (ws, _) = connect_async(&self.ws_url).await?;
            WsConn::new(ws)
        }
    }

    fn reconnect(
        &mut self,
    ) -> impl Future<Output = Result<Self::TransportStream, Self::Error>> + Send + Sync + 'static
    {
        async move {
            let (ws, _) = connect_async(&self.ws_url).await?;
            WsConn::new(ws)
        }
    }
}
