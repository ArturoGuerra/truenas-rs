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

    #[error("command channel closed")]
    CommandChannelClosed,
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
}

pub struct IoTask<S, E>
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

impl<S, E> IoTask<S, E>
where
    E: StdError + Send + Sync + 'static,
    S: Stream<Item = Result<Event, E>> + Sink<Event, Error = E> + Send + Unpin + 'static,
{
    fn as_ctx(&mut self) -> IoCtx<'_, S, E> {
        IoCtx {
            cancel: &self.cancel,
            data_tx: &mut self.data_tx,
            data_rx: &mut self.data_rx,
            event_tx: &mut self.event_tx,
            command_rx: &mut self.command_rx,
        }
    }
}

impl<S, E> IoTask<S, E>
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

    async fn disconnected(ctx: IoCtx<'_, S, E>) -> Result<Next<S, E>, IoError> {
        ctx.event_tx
            .send(IoEvent::ReconnectRequested)
            .await
            .map_err(IoError::EventSend)?;
        Ok(Next::Continue(IoMode::Connecting))
    }

    async fn connecting(ctx: IoCtx<'_, S, E>) -> Result<Next<S, E>, IoError> {
        select! {
            _ = ctx.cancel.cancelled() => {
                Ok(Next::Exit)
            },
            cmd = ctx.command_rx.recv() => match cmd {
                Some(cmd) => match cmd {
                    IoCommand::SwapStream(s) =>
                        Ok(Next::Continue(IoMode::Connected { stream: s, outq: WireOutQueue::new() }))
                    ,
                    _ => {
                        ctx.event_tx.send(IoEvent::ReconnectRequested).await.map_err(IoError::transport_err)?;
                        Ok(Next::Stay)
                    },
                },
                None => Err(IoError::CommandChannelClosed),

            }
        }
    }

    async fn connected(
        ctx: IoCtx<'_, S, E>,
        stream: &mut S,
        outq: &mut WireOutQueue,
    ) -> Result<Next<S, E>, IoError> {
        let (mut sink, mut stream) = stream.split();

        select! {
            _ = ctx.cancel.cancelled() => { Ok(Next::Exit) },
            cmd = ctx.command_rx.recv() => match cmd {
                Some(cmd) => match cmd {
                    IoCommand::SetWriteTimeout(_) => Ok(Next::Stay),
                    IoCommand::SwapStream(s) => {
                        Ok(Next::Continue(IoMode::Connected { stream: s, outq: WireOutQueue::new() }))
                    },
                    IoCommand::PingNow => {
                        Ok(Next::Stay)
                    },
                    IoCommand::Quiesce => {
                        Ok(Next::Stay)
                    },
                    IoCommand::Resume => {
                        Ok(Next::Stay)
                    },

                },
                None => Err(IoError::CommandChannelClosed),
            }
        }
    }

    async fn quiescing(
        ctx: IoCtx<'_, S, E>,
        stream: &mut S,
        outq: &mut WireOutQueue,
    ) -> Result<Next<S, E>, IoError> {
    }

    async fn draining(
        ctx: IoCtx<'_, S, E>,
        stream: &mut Option<S>,
        outq: &mut WireOutQueue,
    ) -> Result<Next<S, E>, IoError> {
    }

    pub async fn run(&mut self) -> Result<(), IoError> {
        loop {
            let ctx = IoCtx {
                cancel: &self.cancel,
                data_tx: &mut self.data_tx,
                data_rx: &mut self.data_rx,
                event_tx: &mut self.event_tx,
                command_rx: &mut self.command_rx,
            };

            let next = match &mut self.mode {
                // We have no stream so we must request one.
                IoMode::Disconnected => IoTask::disconnected(ctx).await?,
                // We are waiting for the supervisor to send a stream.
                IoMode::Connecting => IoTask::connecting(ctx).await?,
                // We are connected have a stream and may be sending/receiving data.
                IoMode::Connected { stream, outq } => IoTask::connected(ctx, stream, outq).await?,
                // We are not accepting new data and flushing our buffer.
                IoMode::Quiescing { stream, outq } => IoTask::quiescing(ctx, stream, outq).await?,
                // We dont have a stream so we cant really flush anything.
                IoMode::Draining { stream, outq } => IoTask::draining(ctx, stream, outq).await?,
            };

            match next {
                Next::Stay => {}
                Next::Continue(mode) => {
                    self.mode = mode;
                    continue;
                }
                Next::Transition(mode) => {
                    self.mode = mode;
                }
                Next::Exit => {
                    break;
                }
            }
        }

        Ok(())
    }
}
