use crate::error::Error;
use crate::transport::Event;
use crate::types::{WireIn, WireInTx, WireOut, WireOutRx};
use bytes::Bytes;
use futures::{Sink, SinkExt, Stream, StreamExt, TryFutureExt};
use std::collections::VecDeque;
use std::error::Error as StdError;
use std::time::Duration;
use tokio::select;
use tokio::sync::mpsc::{self, error::SendError as MpscSendError};
use tokio_util::sync::CancellationToken;

type WireOutQueue = VecDeque<WireOut>;

#[derive(thiserror::Error, Debug)]
pub enum IoError {
    #[error("transport: {0}")]
    Transport(#[from] Box<dyn StdError + Send + Sync>),

    #[error("event sender: {0}")]
    EventSend(#[from] MpscSendError<IoEvent>),
}

impl IoError {
    pub fn transport_err<E>(e: E) -> IoError
    where
        E: StdError + Send + Sync + 'static,
    {
        IoError::Transport(Box::new(e))
    }
}

enum Next<S, E>
where
    E: StdError + Send + Sync + 'static,
    S: Stream<Item = Result<Event, E>> + Sink<Event, Error = E> + Send + Unpin + 'static,
{
    Stay,
    Continue(IoMode<S, E>),
    Transition(IoMode<S, E>),
    Exit,
}

#[derive(Debug)]
pub enum IoMode<S, E>
where
    E: StdError + Send + Sync + 'static,
    S: Stream<Item = Result<Event, E>> + Sink<Event, Error = E> + Send + Unpin + 'static,
{
    Disconnected,
    Connecting,
    Connected {
        stream: S,
        outq: WireOutQueue,
    },
    Quiescing {
        stream: S,
        outq: WireOutQueue,
    },
    Draining {
        stream: Option<S>,
        outq: WireOutQueue,
    },
}

#[derive(Debug)]
pub enum IoEvent {
    Connected,
    Disconnected,
    Backpressured,
    ReconnectRequested,
}

pub enum IoCommand<S, E>
where
    E: StdError + Send + Sync + 'static,
    S: Stream<Item = Result<Event, E>> + Sink<Event, Error = E> + Send + Unpin + 'static,
{
    SetWriteTimeout(Duration),
    SwapStream(S),
    PingNow,
    Quiesce,
    Resume,
}

type IoCommandTx<T, E> = mpsc::Sender<IoCommand<T, E>>;
type IoCommandRx<T, E> = mpsc::Receiver<IoCommand<T, E>>;
type IoEventTx = mpsc::Sender<IoEvent>;
type IoEventRx = mpsc::Receiver<IoEvent>;

struct IoCtx<'a, S, E>
where
    E: StdError + Send + Sync + 'static,
    S: Stream<Item = Result<Event, E>> + Sink<Event, Error = E> + Send + Unpin + 'static,
{
    cancel: &'a CancellationToken,
    data_tx: &'a mut WireInTx,
    data_rx: &'a mut WireOutRx,
    event_tx: &'a mut IoEventTx,
    command_rx: &'a mut IoCommandRx<S, E>,
    mode: &'a mut IoMode<S, E>,
}

pub struct IOTask<S, E>
where
    E: StdError + Send + Sync + 'static,
    S: Stream<Item = Result<Event, E>> + Sink<Event, Error = E> + Send + Unpin + 'static,
{
    cancel: CancellationToken,
    data_tx: WireInTx,
    data_rx: WireOutRx,
    event_tx: IoEventTx,
    command_rx: IoCommandRx<S, E>,
    mode: IoMode<S, E>,
}

impl<S, E> IOTask<S, E>
where
    E: StdError + Send + Sync + 'static,
    S: Stream<Item = Result<Event, E>> + Sink<Event, Error = E> + Send + Unpin + 'static,
{
    pub fn new(
        cancel: CancellationToken,
        data_tx: WireInTx,
        data_rx: WireOutRx,
        event_tx: IoEventTx,
        command_rx: IoCommandRx<S, E>,
        stream: Option<S>,
    ) -> Self {
        let mode = match stream {
            Some(s) => IoMode::Connected {
                stream: s,
                outq: VecDeque::new(),
            },
            None => IoMode::Disconnected,
        };
        Self {
            cancel,
            data_tx,
            data_rx,
            event_tx,
            command_rx,
            mode,
        }
    }

    async fn set_mode(&mut self, mode: IoMode<S, E>) -> Result<(), IoError> {
        self.mode = mode;
        Ok(())
    }

    async fn process_cmd(&mut self, command: Option<IoCommand<S, E>>) -> Result<(), IoError> {
        match command {
            Some(cmd) => match cmd {
                IoCommand::SetWriteTimeout(d) => {}
                IoCommand::SwapStream(new_stream) => {}
                IoCommand::PingNow => {}
                IoCommand::Quiesce => {}
                IoCommand::Resume => {}
            },
            None => {}
        }
        Ok(())
    }

    async fn disconnected(ctx: IoCtx<'_>) -> Result<(), IoError> {
        self.event_tx
            .send(IoEvent::ReconnectRequested)
            .await
            .map_err(IoError::EventSend)?;
        self.set_mode(IoMode::Connecting);
        Ok(())
    }

    async fn connecting(&mut self) -> Result<(), IoError> {
        Ok(())
    }

    pub async fn run(&mut self) -> Result<(), IoError> {
        loop {
            match &mut self.mode {
                // We have no stream so we must request one.
                IoMode::Disconnected => self.disconnected().await?,
                // We are waiting for the supervisor to send a stream.
                IoMode::Connecting => {}
                // We are connected have a stream and may be sending/receiving data.
                IoMode::Connected { .. } => {}
                // We are not accepting new data and flushing our buffer.
                IoMode::Quiescing { .. } => {}
                // We dont have a stream so we cant really flush anything.
                IoMode::Draining { .. } => {}
            }
        }

        Ok(())
    }
}
