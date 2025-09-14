use crate::transport::{Close, Event, TransportRecv, TransportSend};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use tokio_tungstenite::tungstenite::{Error as WsError, Message, protocol::CloseFrame};

impl<T, E> TransportSend for T
where
    T: Sink<Message, Error = E> + Unpin + Send + Sync,
    E: From<WsError> + std::fmt::Debug + Send + 'static,
{
    type Error = E;

    async fn send(&mut self, event: Event) -> Result<(), Self::Error> {
        match event {
            Event::Data(bytes) => SinkExt::send(self, Message::Binary(bytes)).await.unwrap(),
            Event::Ping(bytes) => SinkExt::send(self, Message::Ping(bytes)).await.unwrap(),
            Event::Pong(bytes) => SinkExt::send(self, Message::Pong(bytes)).await.unwrap(),
            Event::Close(Some(close)) => SinkExt::send(
                self,
                Message::Close(Some(CloseFrame {
                    code: (close.code as u16).into(),
                    reason: close.reason.into(),
                })),
            )
            .await
            .unwrap(),
            Event::Close(None) => SinkExt::send(self, Message::Close(None)).await.unwrap(),
        }

        Ok(())
    }
}

impl<T, E> TransportRecv for T
where
    T: Stream<Item = Result<Message, E>> + Unpin + Send + Sync,
    E: From<WsError> + std::fmt::Debug + Send + 'static,
{
    type Error = E;
    async fn recv(&mut self) -> Result<Option<Event>, Self::Error> {
        while let Some(msg) = self.next().await {
            match msg? {
                Message::Binary(bytes) => return Ok(Some(Event::Data(bytes))),
                Message::Text(_) => return Ok(None),
                Message::Ping(bytes) => return Ok(Some(Event::Ping(bytes))),
                Message::Pong(bytes) => return Ok(Some(Event::Pong(bytes))),
                Message::Close(Some(close)) => {
                    return Ok(Some(Event::Close(Some(Close {
                        code: u16::from(close.code) as i64,
                        reason: close.reason.to_string(),
                    }))));
                }
                Message::Close(None) => return Ok(Some(Event::Close(None))),
                _ => {}
            }
        }

        Ok(None)
    }
}
