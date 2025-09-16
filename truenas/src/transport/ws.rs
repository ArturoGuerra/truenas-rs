use crate::transport::{Close, Event, TransportRecv, TransportSend};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use std::error::Error;
use std::fmt::Debug;
use tokio_tungstenite::tungstenite::{Error as WsError, Message, Utf8Bytes, protocol::CloseFrame};

impl<T, E> TransportSend for T
where
    T: Sink<Message, Error = E> + Unpin + Send + Sync,
    E: From<WsError> + Error + Debug + Send + Sync + 'static,
{
    type Error = E;

    async fn send(&mut self, event: Event) -> Result<(), Self::Error> {
        match event {
            Event::Data(bytes) => {
                SinkExt::send(
                    self,
                    Message::Text(unsafe { Utf8Bytes::from_bytes_unchecked(bytes) }),
                )
                .await?
            }
            Event::Ping(bytes) => SinkExt::send(self, Message::Ping(bytes)).await?,
            Event::Pong(bytes) => SinkExt::send(self, Message::Pong(bytes)).await?,
            Event::Close(Some(close)) => {
                SinkExt::send(
                    self,
                    Message::Close(Some(CloseFrame {
                        code: (close.code as u16).into(),
                        reason: close.reason.into(),
                    })),
                )
                .await?
            }
            Event::Close(None) => SinkExt::send(self, Message::Close(None)).await?,
        }

        Ok(())
    }
}

impl<T, E> TransportRecv for T
where
    T: Stream<Item = Result<Message, E>> + Unpin + Send + Sync,
    E: From<WsError> + Error + Debug + Send + Sync + 'static,
{
    type Error = E;
    async fn recv(&mut self) -> Result<Option<Event>, Self::Error> {
        while let Some(msg) = self.next().await {
            match msg? {
                Message::Text(text) => return Ok(Some(Event::Data(text.into()))),
                Message::Ping(bytes) => return Ok(Some(Event::Ping(bytes))),
                Message::Pong(bytes) => return Ok(Some(Event::Pong(bytes))),
                Message::Close(Some(close)) => {
                    return Ok(Some(Event::Close(Some(Close {
                        code: u16::from(close.code) as i64,
                        reason: close.reason.to_string(),
                    }))));
                }
                Message::Close(None) => return Ok(Some(Event::Close(None))),
                Message::Binary(_) | Message::Frame(_) => continue,
            }
        }

        Ok(None)
    }
}
