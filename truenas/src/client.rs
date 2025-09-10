use crate::error::Error;
use crate::types::{
    Cmd, CmdTx, IntoParams, MethodId, MethodIdBuf, RequestId, RequestIdBuf, Result,
    RpcResultPayload, SubscriptionRecv,
};
use serde::de::DeserializeOwned;
use std::borrow::Borrow;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

#[derive(Debug)]
pub struct Response<T: DeserializeOwned> {
    pub id: RequestIdBuf,
    pub result: T,
}

impl<T> Response<T>
where
    T: DeserializeOwned,
{
    pub fn unwrap(self) -> T {
        self.result
    }

    pub fn id(&self) -> &RequestId {
        self.id.as_ref()
    }
}

impl<T> AsRef<RequestId> for Response<T>
where
    T: DeserializeOwned,
{
    fn as_ref(&self) -> &RequestId {
        self.id.as_ref()
    }
}

impl<T> AsRef<T> for Response<T>
where
    T: DeserializeOwned,
{
    fn as_ref(&self) -> &T {
        &self.result
    }
}

impl<T> Borrow<T> for Response<T>
where
    T: DeserializeOwned,
{
    fn borrow(&self) -> &T {
        &self.result
    }
}

// TODO: Impl future
#[derive(Debug)]
pub struct Subscription<T: DeserializeOwned> {
    _marker: std::marker::PhantomData<T>,
    method: Option<MethodIdBuf>,
    cmd_tx: CmdTx,
    recv: Option<SubscriptionRecv>,
}

impl<T> Drop for Subscription<T>
where
    T: DeserializeOwned,
{
    fn drop(&mut self) {
        self.recv.take();

        self.cmd_tx
            .send(Cmd::Unsubscribe {
                method: self.method.take().unwrap(),
            })
            .unwrap();
    }
}

pub struct Client {
    cmd_tx: CmdTx,
}

// the whole client should use an internal thread and loop model that will use channels.
impl Client {
    pub async fn call<T, P>(&self, method: MethodIdBuf, params: P) -> Result<Response<T>>
    where
        T: DeserializeOwned,
        P: IntoParams,
    {
        let id = Uuid::new_v4();
        let (tx, rx) = oneshot::channel();
        let cmd = Cmd::Call {
            id: id.into(),
            method,
            params: params.into_params()?,
            reply: tx,
        };

        self.cmd_tx
            .send(cmd)
            .map_err(|e| Error::TokioSend(e.to_string()))?;

        rx.await
            .map_err(Error::TokioOneshotRecv)?
            .and_then(|payload| {
                Ok(Response {
                    id: payload.id,
                    result: payload.result.deserialize_owned::<T>()?,
                })
            })
    }

    pub async fn notification<P>(&self, method: MethodIdBuf, params: P) -> Result<()>
    where
        P: IntoParams,
    {
        let cmd = Cmd::Notification {
            method,
            params: params.into_params()?,
        };

        self.cmd_tx
            .send(cmd)
            .map_err(|e| Error::TokioSend(e.to_string()))
    }

    pub async fn subscribe<T>(&self, method: MethodIdBuf) -> Result<Subscription<T>>
    where
        T: DeserializeOwned,
    {
        let (tx, rx) = oneshot::channel();
        let cmd = Cmd::Subscribe {
            method: method.clone(),
            ready: tx,
        };

        self.cmd_tx
            .send(cmd)
            .map_err(|e| Error::TokioSend(e.to_string()))?;

        Ok(Subscription {
            _marker: std::marker::PhantomData,
            method: Some(method),
            cmd_tx: self.cmd_tx.clone(),
            recv: Some(rx.await.map_err(Error::TokioOneshotRecv)?),
        })
    }
}
