use crate::error::Error;
use crate::transport::{Event, TransportRecv, TransportSend};
use crate::types::{WireIn, WireInTx, WireOut, WireOutRx};
use bytes::Bytes;
use tokio_util::sync::CancellationToken;

pub async fn read_task<T>(
    wire: WireInTx,
    mut transport: T,
    cancel: CancellationToken,
) -> Result<(), Error>
where
    T: TransportRecv + Send,
{
    while let Some(event) = transport.recv().await.map_err(Error::transport_err)? {
        println!("Got a read event: {:?}", &event);
        match event {
            Event::Data(bytes) => wire
                .send(WireIn::Recv(bytes))
                .map_err(Error::transport_err)?,
            Event::Ping(_) => wire.send(WireIn::Ping).map_err(Error::transport_err)?,
            Event::Pong(_) => wire.send(WireIn::Pong).map_err(Error::transport_err)?,
            Event::Close(_) => {
                wire.send(WireIn::Closed).map_err(Error::transport_err)?;
                break;
            }
        }
    }

    Ok(())
}

pub async fn write_task<T>(
    mut wire: WireOutRx,
    mut transport: T,
    cancel: CancellationToken,
) -> Result<(), Error>
where
    T: TransportSend + Send,
{
    while let Some(wire) = wire.recv().await {
        println!("Got a write event: {:?}", &wire);
        match wire {
            WireOut::Send(bytes) => transport
                .send(Event::Data(bytes))
                .await
                .map_err(Error::transport_err)?,
            WireOut::Ping => transport
                .send(Event::Ping(Bytes::new()))
                .await
                .map_err(Error::transport_err)?,
            WireOut::Pong => transport
                .send(Event::Pong(Bytes::new()))
                .await
                .map_err(Error::transport_err)?,
            WireOut::Close => {
                transport
                    .send(Event::Close(None))
                    .await
                    .map_err(Error::transport_err)?;
                break;
            }
        }
    }

    Ok(())
}
