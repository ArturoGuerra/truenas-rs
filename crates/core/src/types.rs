use bytes::Bytes;

#[derive(Debug)]
pub enum WireIn {
    Recv(Bytes),
    Closed,
}

#[derive(Debug)]
pub enum WireOut {
    Send(Bytes),
    Close,
}
