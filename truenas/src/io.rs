use crate::error::Error;
use crate::transport::Event;
use crate::transport::Transport;
use crate::types::{WireIn, WireInTx, WireOut, WireOutRx};
use bytes::Bytes;
use futures::{Sink, SinkExt, Stream, StreamExt};
use tokio_util::sync::CancellationToken;
use std::error::Error as StdError;

pub async fn io_task<T, E>(
    wire_in: WireInTx,
    mut wire_out: WireOutRx,
    stream: T,
    cancel: CancellationToken,
) -> Result<(), Error>
where
    E: StdError + Send + Sync + 'static, 
    T: Stream<Item = Result<Event, E>
        + Sink<Event, Error = E>
        + Send
        + Sync
        + Unpin
        + 'static,
{
    let (mut sink, mut stream) = stream.split();

    enum StreamStatus {
        Close,
        Continue,
    }

    let stream_select = async move |cancel: CancellationToken| -> Result<StreamStatus, Error> {
        tokio::select! {
             t = stream.next() => match t.transpose().map_err(Error::transport_err)? {
                 Some(event) => match event {
                     Event::Data(bytes) => wire_in
                         .send(WireIn::Recv(bytes))
                         .map_err(Error::transport_err)?,
                     Event::Ping(_) => wire_in.send(WireIn::Ping).map_err(Error::transport_err)?,
                     Event::Pong(_) => wire_in.send(WireIn::Pong).map_err(Error::transport_err)?,
                     Event::Close(_) => {
                        wire_in.send(WireIn::Closed).map_err(Error::transport_err)?;

                        return Ok(StreamStatus::Close);
                    },
                 },
                 None => { return Ok(StreamStatus::Close); },
             },

             w = wire_out.recv() => match w {
                 Some(wire) => {
                     match wire {
                         WireOut::Send(bytes) => sink
                             .send(Event::Data(bytes))
                             .await
                             .map_err(Error::transport_err)?,
                         WireOut::Ping => sink
                             .send(Event::Ping(Bytes::new()))
                             .await
                             .map_err(Error::transport_err)?,
                         WireOut::Pong => sink
                             .send(Event::Pong(Bytes::new()))
                             .await
                             .map_err(Error::transport_err)?,
                         WireOut::Close => {
                              sink
                                 .send(Event::Close(None))
                                 .await
                                 .map_err(Error::transport_err)?;
                             cancel.cancel();
                             return Ok(StreamStatus::Close);
                         }
                     }
                 },
                 None => {  return Ok(StreamStatus::Close); }
            }
        }

        Ok(StreamStatus::Continue)
    };

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => { return Ok(()); },
            _ = async {
                    stream_select.await
            } => { }

        }
    }
}
