//! Windows PTY I/O actor.
//!
//! Mirrors the Unix actor's public surface on top of portable-pty's
//! native ConPTY master. Instead of `poll()` over a master fd, a blocking
//! reader thread forwards ConPTY output while the actor thread serially
//! runs the terminal feed callback, drains user input, applies resizes,
//! and writes terminal responses.
//!
//! User input is buffered until the first ConPTY output arrives, which
//! proves the shell is alive and reading; input sent before the shell
//! boots would otherwise be lost on Windows because nothing queues it.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use portable_pty::{MasterPty, PtySize};
use tokio::sync::mpsc;
use tracing::{debug, warn};

const ACTOR_TICK: Duration = Duration::from_millis(20);
const ACTOR_COMMAND_BUFFER: usize = 1024;
/// Upper bound for user input buffered while the shell has not booted yet.
const PENDING_INPUT_CAP: usize = 256 * 1024;

pub(crate) struct PtyReadResult {
    pub terminal_responses: Vec<Bytes>,
}

type ReadCallback = Box<dyn FnMut(&[u8]) -> PtyReadResult + Send + 'static>;
type ReaderExitCallback = Box<dyn FnOnce() + Send + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PtyResize {
    rows: u16,
    cols: u16,
    cell_width_px: u32,
    cell_height_px: u32,
}

impl PtyResize {
    fn to_pty_size(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: self.cell_width_px as u16,
            pixel_height: self.cell_height_px as u16,
        }
    }
}

#[derive(Debug)]
struct UserWriteGate {
    accepting: bool,
}

pub(crate) struct PtyIoActorConfig {
    pub pane_id: u32,
    pub master: Box<dyn MasterPty + Send>,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn std::io::Write + Send>,
    pub initially_quiesced: bool,
    pub on_read: ReadCallback,
    pub on_reader_exit: Option<ReaderExitCallback>,
}

enum ReaderEvent {
    Data(Vec<u8>),
    Eof,
}

enum ControlCommand {
    Resize {
        resize: PtyResize,
        terminal_responses: Vec<Bytes>,
    },
    Nudge(PtyResize),
    Shutdown,
}

#[derive(Clone)]
pub(crate) struct PtyIoActorHandle {
    data_tx: mpsc::Sender<Bytes>,
    control_tx: std_mpsc::Sender<ControlCommand>,
    user_writes: Arc<Mutex<UserWriteGate>>,
}

impl PtyIoActorHandle {
    pub(crate) async fn write_user_input(
        &self,
        bytes: Bytes,
    ) -> Result<(), mpsc::error::SendError<Bytes>> {
        if bytes.is_empty() {
            return Ok(());
        }
        if !self.user_input_accepted() {
            return Err(mpsc::error::SendError(bytes));
        }
        let permit = match self.data_tx.reserve().await {
            Ok(permit) => permit,
            Err(_) => return Err(mpsc::error::SendError(bytes)),
        };
        if !self.user_input_accepted() {
            return Err(mpsc::error::SendError(bytes));
        }
        permit.send(bytes);
        Ok(())
    }

    pub(crate) fn try_write_user_input(
        &self,
        bytes: Bytes,
    ) -> Result<(), mpsc::error::TrySendError<Bytes>> {
        if bytes.is_empty() {
            return Ok(());
        }
        if !self.user_input_accepted() {
            return Err(mpsc::error::TrySendError::Full(bytes));
        }
        self.data_tx.try_send(bytes)
    }

    fn user_input_accepted(&self) -> bool {
        let user_writes = self
            .user_writes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        user_writes.accepting
    }

    pub(crate) fn shutdown(&self) {
        {
            let mut user_writes = self
                .user_writes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            user_writes.accepting = false;
        }
        let _ = self.control_tx.send(ControlCommand::Shutdown);
    }

    pub(crate) fn resize(
        &self,
        rows: u16,
        cols: u16,
        cell_width_px: u32,
        cell_height_px: u32,
        terminal_responses: Vec<Bytes>,
    ) {
        let _ = self.control_tx.send(ControlCommand::Resize {
            resize: PtyResize {
                rows,
                cols,
                cell_width_px,
                cell_height_px,
            },
            terminal_responses,
        });
    }

    pub(crate) fn nudge_child_redraw_after_handoff(
        &self,
        rows: u16,
        cols: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    ) {
        let _ = self.control_tx.send(ControlCommand::Nudge(PtyResize {
            rows,
            cols,
            cell_width_px,
            cell_height_px,
        }));
    }
}

pub(crate) struct PtyIoActor;

impl PtyIoActor {
    pub(crate) fn spawn(config: PtyIoActorConfig) -> std::io::Result<PtyIoActorHandle> {
        let (data_tx, data_rx) = mpsc::channel(ACTOR_COMMAND_BUFFER);
        let (control_tx, control_rx) = std_mpsc::channel();
        let user_writes = Arc::new(Mutex::new(UserWriteGate {
            accepting: !config.initially_quiesced,
        }));
        let handle = PtyIoActorHandle {
            data_tx,
            control_tx,
            user_writes: Arc::clone(&user_writes),
        };

        let (reader_tx, reader_rx) = std_mpsc::channel::<ReaderEvent>();
        spawn_reader_thread(config.pane_id, config.reader, reader_tx);

        let mut runner = PtyIoActorRunner {
            pane_id: config.pane_id,
            master: config.master,
            writer: config.writer,
            data_rx,
            control_rx,
            reader_rx,
            on_read: config.on_read,
            on_reader_exit: config.on_reader_exit,
            pending_input: VecDeque::new(),
            pending_input_bytes: 0,
            shell_booted: false,
        };
        std::thread::Builder::new()
            .name(format!("herdr-pty-{}", config.pane_id))
            .spawn(move || runner.run())
            .map_err(|err| std::io::Error::other(err.to_string()))?;

        Ok(handle)
    }
}

fn spawn_reader_thread(
    pane_id: u32,
    mut reader: Box<dyn Read + Send>,
    reader_tx: std_mpsc::Sender<ReaderEvent>,
) {
    let spawned = std::thread::Builder::new()
        .name(format!("herdr-pty-reader-{pane_id}"))
        .spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        let _ = reader_tx.send(ReaderEvent::Eof);
                        break;
                    }
                    Ok(len) => {
                        if reader_tx
                            .send(ReaderEvent::Data(buffer[..len].to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(err) => {
                        debug!(pane = pane_id, err = %err, "PTY reader failed");
                        let _ = reader_tx.send(ReaderEvent::Eof);
                        break;
                    }
                }
            }
        });
    // The join handle is dropped on purpose: the reader thread is detached
    // and exits when ConPTY closes the output pipe after the child dies.
    if let Err(err) = spawned {
        warn!(pane = pane_id, err = %err, "failed to spawn PTY reader thread");
    }
}

struct PtyIoActorRunner {
    pane_id: u32,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    data_rx: mpsc::Receiver<Bytes>,
    control_rx: std_mpsc::Receiver<ControlCommand>,
    reader_rx: std_mpsc::Receiver<ReaderEvent>,
    on_read: ReadCallback,
    on_reader_exit: Option<ReaderExitCallback>,
    /// User input received before the first ConPTY output, kept in order
    /// until the shell proves it is reading.
    pending_input: VecDeque<Vec<u8>>,
    pending_input_bytes: usize,
    /// Flips to true on the first `ReaderEvent::Data` so buffered input is
    /// flushed and later input is written straight to the PTY.
    shell_booted: bool,
}

impl PtyIoActorRunner {
    fn run(&mut self) {
        let mut should_exit = false;
        while !should_exit {
            self.drain_user_input();
            should_exit = self.drain_controls();

            match self.reader_rx.recv_timeout(ACTOR_TICK) {
                Ok(ReaderEvent::Data(bytes)) => {
                    if !self.shell_booted {
                        self.shell_booted = true;
                        self.flush_pending_input();
                    }
                    self.handle_output(&bytes);
                }
                Ok(ReaderEvent::Eof) => break,
                Err(std_mpsc::RecvTimeoutError::Timeout) => {}
                Err(std_mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        if let Some(on_reader_exit) = self.on_reader_exit.take() {
            on_reader_exit();
        }
    }

    fn drain_user_input(&mut self) {
        while let Ok(bytes) = self.data_rx.try_recv() {
            if bytes.is_empty() {
                continue;
            }
            if !self.shell_booted {
                self.queue_pending_input(&bytes);
                continue;
            }
            if let Err(err) = self.writer.write_all(&bytes) {
                warn!(pane = self.pane_id, err = %err, "PTY write of user input failed");
                return;
            }
            let _ = self.writer.flush();
        }
    }

    /// Buffers input received while the shell has not booted yet, dropping
    /// the oldest bytes once the pending queue exceeds its cap.
    fn queue_pending_input(&mut self, bytes: &[u8]) {
        self.pending_input_bytes += bytes.len();
        self.pending_input.push_back(bytes.to_vec());
        let mut dropped = 0usize;
        while self.pending_input_bytes > PENDING_INPUT_CAP {
            match self.pending_input.pop_front() {
                Some(oldest) => {
                    self.pending_input_bytes -= oldest.len();
                    dropped += oldest.len();
                }
                None => break,
            }
        }
        if dropped > 0 {
            warn!(
                pane = self.pane_id,
                bytes = dropped,
                "pending PTY input over cap, dropped oldest buffered input"
            );
        }
    }

    /// Writes the buffered pre-boot input to the PTY in arrival order.
    fn flush_pending_input(&mut self) {
        if self.pending_input.is_empty() {
            return;
        }
        let queued = std::mem::take(&mut self.pending_input);
        self.pending_input_bytes = 0;
        for bytes in &queued {
            if let Err(err) = self.writer.write_all(bytes) {
                warn!(
                    pane = self.pane_id,
                    err = %err,
                    "PTY write of pending user input failed"
                );
                return;
            }
        }
        let _ = self.writer.flush();
    }

    /// Returns true when the actor received a shutdown command.
    fn drain_controls(&mut self) -> bool {
        while let Ok(command) = self.control_rx.try_recv() {
            match command {
                ControlCommand::Resize {
                    resize,
                    terminal_responses,
                } => {
                    self.apply_resize(resize);
                    for response in terminal_responses {
                        if response.is_empty() {
                            continue;
                        }
                        if let Err(err) = self.writer.write_all(&response) {
                            debug!(pane = self.pane_id, err = %err, "terminal response write failed");
                        }
                    }
                }
                ControlCommand::Nudge(resize) => self.apply_nudge(resize),
                ControlCommand::Shutdown => return true,
            }
        }
        false
    }

    fn handle_output(&mut self, bytes: &[u8]) {
        let result = (self.on_read)(bytes);
        for response in result.terminal_responses {
            if response.is_empty() {
                continue;
            }
            if let Err(err) = self.writer.write_all(&response) {
                debug!(pane = self.pane_id, err = %err, "terminal response write failed");
            }
        }
    }

    fn apply_resize(&mut self, resize: PtyResize) {
        if let Err(err) = self.master.resize(resize.to_pty_size()) {
            debug!(pane = self.pane_id, err = %err, "PTY resize failed");
        }
    }

    fn apply_nudge(&mut self, resize: PtyResize) {
        let nudge = if resize.rows > 2 {
            PtyResize {
                rows: resize.rows - 1,
                ..resize
            }
        } else {
            PtyResize {
                cols: resize.cols.saturating_sub(1).max(4),
                ..resize
            }
        };
        if nudge == resize {
            return;
        }
        self.apply_resize(nudge);
        std::thread::sleep(Duration::from_millis(30));
        self.apply_resize(resize);
    }
}
