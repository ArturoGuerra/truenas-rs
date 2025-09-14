use bytes::Bytes;

use crate::transport::{Event, TransportRecv, TransportSend};
use crate::types::{WireIn, WireInTx, WireOut, WireOutRx};

pub async fn read_task<T>(wire: WireInTx, transport: T)
where
    T: TransportRecv + Send,
{
    while let Some(event) = transport.recv().await.unwrap() {
        match event {
            Event::Data(bytes) => wire.send(WireIn::Recv(bytes)).unwrap(),
            Event::Ping(_) => wire.send(WireIn::Ping).unwrap(),
            Event::Pong(_) => wire.send(WireIn::Pong).unwrap(),
            Event::Close(_) => wire.send(WireIn::Closed).unwrap(),
        }
    }
}

pub async fn write_task<T>(mut wire: WireOutRx, transport: T)
where
    T: TransportSend + Send,
{
    loop {
        match wire.recv().await.unwrap() {
            WireOut::Send(bytes) => transport.send(bytes).await.unwrap(),
            WireOut::Ping => transport.ping(Bytes::new()).await.unwrap(),
            WireOut::Pong => transport.pong(Bytes::new()).await.unwrap(),
            WireOut::Close => {
                transport.close().await.unwrap();
                break;
            }
        }
    }
}
