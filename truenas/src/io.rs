use crate::{
    transport::Event,
    types::{WireIn, WireInTx, WireOut, WireOutRx},
};
use futures::{
    Sink, SinkExt, Stream, StreamExt,
    stream::{SplitSink, SplitStream},
};
use futures_util::future::{AbortHandle, Abortable};
use std::{collections::VecDeque, error::Error as StdError, mem, time::Duration};
use tokio::{
    select,
    sync::mpsc::{self, error::SendError as MpscSendError},
};
use tokio_util::sync::CancellationToken;

#[derive(thiserror::Error, Debug)]
pub enum IoError {
    #[error("transport: {0}")]
    Transport(#[from] Box<dyn StdError + Send + Sync>),

    #[error("event sender: {0}")]
    EventSend(#[from] MpscSendError<IoEvent>),

    #[error("command channel closed")]
    CommandChannelClosed,

    #[error("wire in sender: {0}")]
    WireInSend(#[from] MpscSendError<WireIn>),
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
    Exit,
    Stay,
    Draining,
    Connecting,
    Connected(S),
    Disconnected,
    SetTimeout(Duration),
}

#[derive(Debug)]
pub enum IoMode<S, E>
where
    E: StdError + Send + Sync + 'static,
    S: Stream<Item = Result<Event, E>> + Sink<Event, Error = E> + Send + Unpin + 'static,
{
    Disconnected,
    Connecting,
    Connected(S),
    Draining(S),
}

impl<S, E> Default for IoMode<S, E>
where
    E: StdError + Send + Sync + 'static,
    S: Stream<Item = Result<Event, E>> + Sink<Event, Error = E> + Send + Unpin + 'static,
{
    fn default() -> Self {
        IoMode::Disconnected
    }
}

#[derive(Debug)]
pub enum IoEvent {
    Connected,
    Disconnected,
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
    timeout: &'a Duration,
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
    timeout: Duration,
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
            timeout: &self.timeout,
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
        timeout: Duration,
        stream: Option<S>,
    ) -> Self {
        let mode = match stream {
            Some(stream) => IoMode::Connected(stream),
            None => IoMode::Disconnected,
        };
        Self {
            cancel,
            data_tx,
            data_rx,
            event_tx,
            command_rx,
            timeout,
            mode,
        }
    }

    pub async fn run(&mut self) -> Result<(), IoError> {
        loop {
            let ctx = IoCtx {
                cancel: &self.cancel,
                data_tx: &mut self.data_tx,
                data_rx: &mut self.data_rx,
                event_tx: &mut self.event_tx,
                command_rx: &mut self.command_rx,
                timeout: &self.timeout,
            };

            let next = match &mut self.mode {
                // We have no stream so we must request one.
                IoMode::Disconnected => IoTask::disconnected(ctx).await?,
                // We are waiting for the supervisor to send a stream.
                IoMode::Connecting => IoTask::connecting(ctx).await?,
                // We are connected have a stream and may be sending/receiving data.
                IoMode::Connected(stream) => IoTask::connected(ctx, stream).await?,
                // We we are draining all remaining data to shutdown.
                IoMode::Draining(stream) => IoTask::draining(ctx, stream).await?,
            };

            match next {
                Next::Stay => {
                    continue;
                }
                Next::Exit => {
                    break;
                }
                Next::Connecting => self.mode = IoMode::Connecting,
                Next::Disconnected => self.mode = IoMode::Disconnected,
                Next::Connected(stream) => self.mode = IoMode::Connected(stream),
                Next::Draining => {
                    if matches!(self.mode, IoMode::Connected { .. }) {
                        self.mode = match mem::take(&mut self.mode) {
                            IoMode::Connected(stream) => IoMode::Draining(stream),
                            _ => break,
                        }
                    }
                }
                Next::SetTimeout(duration) => {
                    self.timeout = duration;
                }
            }
        }

        Ok(())
    }

    async fn command(
        ctx: &IoCtx<'_, S, E>,
        command: Option<IoCommand<S, E>>,
    ) -> Result<Next<S, E>, IoError> {
        match command {
            Some(command) => match command {
                IoCommand::PingNow => Ok(Next::Stay),
                IoCommand::SwapStream(stream) => Ok(Next::Connected(stream)),
                IoCommand::SetWriteTimeout(duration) => Ok(Next::SetTimeout(duration)),
            },
            None => Ok(Next::Exit),
        }
    }

    async fn disconnected(ctx: IoCtx<'_, S, E>) -> Result<Next<S, E>, IoError> {
        ctx.event_tx
            .send(IoEvent::ReconnectRequested)
            .await
            .map_err(IoError::EventSend)?;
        Ok(Next::Connecting)
    }

    async fn connecting(ctx: IoCtx<'_, S, E>) -> Result<Next<S, E>, IoError> {
        select! {
            _ = ctx.cancel.cancelled() => {
                Ok(Next::Exit)
            },
            cmd = ctx.command_rx.recv() => Self::command(&ctx, cmd).await,

        }
    }

    async fn draining(ctx: IoCtx<'_, S, E>, stream: &mut S) -> Result<Next<S, E>, IoError> {
        select! {
            _ = ctx.cancel.cancelled() => Ok(Next::Exit),
            cmd = ctx.command_rx.recv() => Self::command(&ctx, cmd).await,
        }
    }

    async fn connected(ctx: IoCtx<'_, S, E>, stream: &mut S) -> Result<Next<S, E>, IoError> {
        let (mut sink, mut stream) = stream.split();

        let (r_abort, r_reg) = AbortHandle::new_pair();
        let r_task = Abortable::new(Self::reader(ctx.data_tx, stream), r_reg);
        let mut r_done = Box::pin(r_task);

        let (w_abort, w_reg) = AbortHandle::new_pair();
        let w_task = Abortable::new(Self::writer(ctx.data_rx, sink), w_reg);
        let mut w_done = Box::pin(w_task);

        loop {
            select! {
                _ = ctx.cancel.cancelled() => { return Ok(Next::Draining) },
                command = ctx.command_rx.recv() => return match command {
                    Some(command) => match command {
                        IoCommand::PingNow => Ok(Next::Stay),
                        IoCommand::SwapStream(stream) => Ok(Next::Connected(stream)),
                        IoCommand::SetWriteTimeout(duration) => Ok(Next::SetTimeout(duration)),
                    },
                    None => Ok(Next::Exit),
                },
                res = &mut r_done => {
                },
                res = &mut w_done => {
                }
            }
        }
    }

    async fn writer(
        data_rx: &mut WireOutRx,
        mut sink: SplitSink<&mut S, Event>,
    ) -> Result<(), IoError> {
        loop {
            match data_rx.recv().await {
                Some(WireOut::Data(bytes)) => sink
                    .send(Event::Data(bytes))
                    .await
                    .map_err(IoError::transport_err)?,
                None => return Ok(()),
            }
        }
    }

    async fn reader(
        event_tx: &mut IoEventTx,
        data_tx: &mut WireInTx,
        mut stream: SplitStream<&mut S>,
    ) -> Result<Next<S, E>, IoError> {
        loop {
            match stream.next().await {
                Some(Ok(Event::Data(bytes))) => {
                    data_tx
                        .send(WireIn::Data(bytes))
                        .await
                        .map_err(IoError::WireInSend)?;
                }
                Some(Ok(Event::Ping(_))) | Some(Ok(Event::Pong(_))) => {
                    unreachable!()
                }

                Some(Ok(Event::Close(close))) => {
                    // TODO: Redo this with logging framework.
                    if let Some(close) = close {
                        println!("stream closed: {:?}", close);
                    }
                    event_tx
                        .send(IoEvent::Disconnected)
                        .await
                        .map_err(IoError::EventSend)?;
                    return Ok(Next::Disconnected);
                }

                Some(Err(err)) => return Err(IoError::transport_err(err)),

                None => {
                    event_tx
                        .send(IoEvent::Disconnected)
                        .await
                        .map_err(IoError::EventSend)?;
                    return Ok(Next::Disconnected);
                }
            }
        }
    }
}
