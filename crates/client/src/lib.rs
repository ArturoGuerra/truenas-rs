pub struct Client {}

// the whole client should use an internal thread and loop model that will use channels.
impl Client {
    async fn connect(url: impl AsRef<str>) {}

    async fn call<T>() {}

    // Returns a stream? or something that tracks the job, should clone the inner client to query
    // the api on its own.
    async fn job<T>() {}

    // Returns a stream that outputs subscriptions and or lets you unsub.
    async fn subscribe<T>() {}
}
