use crate::error::Error;

use bytes::Bytes;

use crate::transport::{Event, TransportRecv, TransportSend};
use crate::types::{WireIn, WireInTx, WireOut, WireOutRx};

pub async fn read_task<T>(wire: WireInTx, mut transport: T) -> Result<(), Error>
where
    T: TransportRecv + Send,
{
    while let Some(event) = transport.recv().await.map_err(Error::from)? {
        match event {
            Event::Data(bytes) => wire.send(WireIn::Recv(bytes)).map_err(Error::from)?,
            Event::Ping(_) => wire.send(WireIn::Ping).map_err(Error::from)?,
            Event::Pong(_) => wire.send(WireIn::Pong).map_err(Error::from)?,
            Event::Close(_) => wire.send(WireIn::Closed).map_err(Error::from)?,
        }
    }

    Ok(())
}

pub async fn write_task<T>(mut wire: WireOutRx, mut transport: T) -> Result<(), Error>
where
    T: TransportSend + Send,
{
    loop {
        match wire.recv().await.map_err(Error::from)? {
            WireOut::Send(bytes) => transport
                .send(Event::Data(bytes))
                .await
                .map_err(Error::from)?,
            WireOut::Ping => transport
                .send(Event::Ping(Bytes::new()))
                .await
                .map_err(Error::from)?,
            WireOut::Pong => transport
                .send(Event::Pong(Bytes::new()))
                .await
                .map_err(Error::from)?,
            WireOut::Close => {
                transport
                    .send(Event::Close(None))
                    .await
                    .map_err(Error::from)?;
                return Ok(());
            }
        }
    }
}
