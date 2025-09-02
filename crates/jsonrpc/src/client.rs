use crate::protocol::*;

struct Notifier;

trait Client {
    async fn request<T, Params>(
        &self,
        method: impl AsRef<str>,
        params: Params,
    ) -> Result<RpcResponse<T>, Error>;

    async fn notification(&self, method: impl AsRef<str>) -> Result<Notifier, Error>;
}
