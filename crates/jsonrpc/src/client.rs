use crate::error::Error;
use crate::protocol::Params;
use serde::de::DeserializeOwned;

#[derive(Clone, Debug)]
struct RequestManager {}

#[derive(Debug)]
pub struct Subscriber<T>
where
    T: DeserializeOwned,
{
    _marker: std::marker::PhantomData<T>,
}

#[derive(Debug)]
pub struct Response<T>
where
    T: DeserializeOwned,
{
    id: String,
    result: T,
}

trait Client {
    async fn request<T, P>(&self, method: impl AsRef<str>, params: P) -> Result<Response<T>, Error>
    where
        T: DeserializeOwned;

    // Sends a notification
    async fn notify(&self, method: impl AsRef<str>, params: Params) -> Result<(), Error>;

    // Subscribes to a notification
    async fn subscribe<T>(&self, method: impl AsRef<str>) -> Result<Subscriber<T>, Error>
    where
        T: DeserializeOwned;
}
