// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The hub: one room per open document, created on first join.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};
use vellum_kernel::{DocId, Error, ErrorKind, Result, SpaceId, UserId};

use crate::config::CollabConfig;
use crate::metrics::Metrics;
use crate::ports::{DocumentFiles, MarkdownCodec, UpdateStore};
use crate::room::{self, Command, Control, JoinOutcome};
use crate::trace::TraceParent;

/// Joins to retry when a room closes between lookup and join.
const JOIN_ATTEMPTS: usize = 3;

/// Identifies a room: one per document. The space is the shard key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoomKey {
    /// The space that owns the document.
    pub space: SpaceId,
    /// The document.
    pub doc: DocId,
}

impl fmt::Display for RoomKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.space, self.doc)
    }
}

/// What a connection may do in a room. There is no "none": a caller
/// without access is refused before it reaches the hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Access {
    /// Sees edits and presence; its updates are refused.
    Read,
    /// Edits.
    Write,
}

impl Access {
    /// Whether updates from this connection are applied.
    #[must_use]
    pub const fn can_write(self) -> bool {
        matches!(self, Self::Write)
    }
}

/// A request to join a room, made after authentication and authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinRequest {
    /// The room.
    pub room: RoomKey,
    /// The signed-in user.
    pub user: UserId,
    /// What authorization granted.
    pub access: Access,
    /// The caller's trace context, from the upgrade request.
    pub traceparent: Option<TraceParent>,
}

/// Why the server closed a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloseReason {
    /// The user's access was revoked.
    Revoked,
    /// The editor did not keep up with the room's updates.
    SlowConsumer,
    /// The editor sent a malformed message.
    Protocol,
    /// The editor sent a frame over the size limit.
    TooLarge,
    /// The server is draining; reconnect to another node.
    Shutdown,
}

impl CloseReason {
    /// The WebSocket close code: the standard one where it exists, else a
    /// code in the application range.
    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::Protocol => 1002,
            Self::TooLarge => 1009,
            Self::Shutdown => 1012,
            Self::Revoked => 4403,
            Self::SlowConsumer => 4408,
        }
    }

    /// A short label for the close frame, logs and metrics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Revoked => "revoked",
            Self::SlowConsumer => "slow_consumer",
            Self::Protocol => "protocol",
            Self::TooLarge => "too_large",
            Self::Shutdown => "shutdown",
        }
    }
}

/// Something the room sends to one connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outbound {
    /// A binary frame of protocol messages.
    Frame(Bytes),
    /// The room closed the connection; nothing follows.
    Close(CloseReason),
}

/// Identifies a connection inside its room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PeerId(pub u64);

/// Sends a connection's frames to its room; dropping it leaves the room.
#[derive(Debug)]
pub struct PeerSender {
    peer: PeerId,
    cmd: mpsc::Sender<Command>,
    ctl: mpsc::UnboundedSender<Control>,
}

impl PeerSender {
    /// Delivers one frame from the editor. Waits while the room is busy,
    /// which pushes back on a fast sender.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unavailable`] when the room has closed.
    pub async fn send(&self, frame: Vec<u8>) -> Result<()> {
        self.cmd
            .send(Command::Message {
                peer: self.peer,
                frame,
            })
            .await
            .map_err(|_| room_gone())
    }
}

impl Drop for PeerSender {
    fn drop(&mut self) {
        let _closed = self.ctl.send(Control::Leave(self.peer));
    }
}

/// A connection to a room: frames go in through [`PeerSender`] and come
/// out of the outbound queue.
#[derive(Debug)]
pub struct Connection {
    sender: PeerSender,
    outbound: mpsc::Receiver<Outbound>,
}

impl Connection {
    /// Delivers one frame from the editor; see [`PeerSender::send`].
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unavailable`] when the room has closed.
    pub async fn send(&self, frame: Vec<u8>) -> Result<()> {
        self.sender.send(frame).await
    }

    /// The next thing to send to the editor; `None` once the room has
    /// dropped this connection.
    pub async fn recv(&mut self) -> Option<Outbound> {
        self.outbound.recv().await
    }

    /// Splits the connection so reading and writing can run apart.
    #[must_use]
    pub fn into_parts(self) -> (PeerSender, mpsc::Receiver<Outbound>) {
        (self.sender, self.outbound)
    }
}

/// What every room shares.
pub struct Deps {
    pub store: Arc<dyn UpdateStore>,
    pub files: Arc<dyn DocumentFiles>,
    pub codec: Arc<dyn MarkdownCodec>,
    pub config: CollabConfig,
    pub metrics: Metrics,
}

/// The open rooms of this process.
#[derive(Default)]
pub struct Registry {
    rooms: Mutex<HashMap<RoomKey, RoomEntry>>,
    next_generation: AtomicU64,
}

#[derive(Clone)]
pub struct RoomEntry {
    pub generation: u64,
    pub cmd: mpsc::Sender<Command>,
    pub ctl: mpsc::UnboundedSender<Control>,
}

impl Registry {
    pub fn lock(&self) -> MutexGuard<'_, HashMap<RoomKey, RoomEntry>> {
        self.rooms.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Removes the entry if it is still the given generation.
    pub fn remove(&self, room: RoomKey, generation: u64) {
        let mut rooms = self.lock();
        if rooms.get(&room).is_some_and(|e| e.generation == generation) {
            let _removed = rooms.remove(&room);
        }
    }
}

/// The collaboration service of one process. Cheap to clone.
#[derive(Clone)]
pub struct Hub {
    deps: Arc<Deps>,
    registry: Arc<Registry>,
    next_peer: Arc<AtomicU64>,
}

impl fmt::Debug for Hub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hub")
            .field("rooms", &self.open_rooms())
            .field("config", &self.deps.config)
            .finish_non_exhaustive()
    }
}

impl Hub {
    /// A hub over the given ports.
    #[must_use]
    pub fn new(
        store: Arc<dyn UpdateStore>,
        files: Arc<dyn DocumentFiles>,
        codec: Arc<dyn MarkdownCodec>,
        config: CollabConfig,
    ) -> Self {
        Self {
            deps: Arc::new(Deps {
                store,
                files,
                codec,
                config,
                metrics: Metrics::new(),
            }),
            registry: Arc::default(),
            next_peer: Arc::new(AtomicU64::new(1)),
        }
    }

    /// The configuration rooms run with.
    #[must_use]
    pub fn config(&self) -> &CollabConfig {
        &self.deps.config
    }

    /// Joins a room, opening it from storage if needed. The room greets
    /// the connection with sync step 1, the awareness states and the peer
    /// list.
    ///
    /// # Errors
    ///
    /// When the room cannot be loaded from storage.
    pub async fn join(&self, request: JoinRequest) -> Result<Connection> {
        for _ in 0..JOIN_ATTEMPTS {
            let entry = self.room(request.room);
            let peer = PeerId(self.next_peer.fetch_add(1, Ordering::Relaxed));
            let (tx, outbound) = mpsc::channel(self.deps.config.peer_queue.max(1));
            let (reply, joined) = oneshot::channel();
            let command = Command::Join {
                peer,
                request: request.clone(),
                tx,
                reply,
            };
            if entry.cmd.send(command).await.is_err() {
                self.registry.remove(request.room, entry.generation);
                continue;
            }
            match joined.await {
                Ok(JoinOutcome::Joined) => {
                    return Ok(Connection {
                        sender: PeerSender {
                            peer,
                            cmd: entry.cmd,
                            ctl: entry.ctl,
                        },
                        outbound,
                    });
                }
                Ok(JoinOutcome::Failed(err)) => return Err(err),
                Ok(JoinOutcome::Retry) | Err(_) => {
                    self.registry.remove(request.room, entry.generation);
                }
            }
        }
        Err(room_gone())
    }

    /// Writes a Markdown version of an open room now, as if an editor had
    /// pressed Done. Does nothing when the room is not open: its file is
    /// already current.
    ///
    /// # Errors
    ///
    /// When the version cannot be written.
    pub async fn done(&self, room: RoomKey) -> Result<()> {
        let Some(entry) = self.registry.lock().get(&room).cloned() else {
            return Ok(());
        };
        let (reply, result) = oneshot::channel();
        if entry.cmd.send(Command::Done { reply }).await.is_err() {
            return Ok(());
        }
        result.await.unwrap_or(Ok(()))
    }

    /// Applies an external edit of the document's Markdown (from the
    /// watcher's three-way merge) to the open room, as a CRDT edit so
    /// concurrent typing survives. Returns `false` when the room is not
    /// open, in which case the caller writes the file itself.
    ///
    /// # Errors
    ///
    /// When the Markdown cannot be converted.
    pub async fn apply_markdown(&self, room: RoomKey, markdown: String) -> Result<bool> {
        let Some(entry) = self.registry.lock().get(&room).cloned() else {
            return Ok(false);
        };
        let (reply, result) = oneshot::channel();
        if entry
            .cmd
            .send(Command::ApplyMarkdown { markdown, reply })
            .await
            .is_err()
        {
            return Ok(false);
        }
        match result.await {
            Ok(Ok(())) => Ok(true),
            Ok(Err(err)) => Err(err),
            Err(_) => Ok(false),
        }
    }

    /// Changes a user's access to an open room at once: `None` disconnects
    /// their connections, `Read` stops applying their edits.
    pub fn set_access(&self, room: RoomKey, user: UserId, access: Option<Access>) {
        if let Some(entry) = self.registry.lock().get(&room) {
            let _closed = entry.ctl.send(Control::SetAccess { user, access });
        }
    }

    /// How many rooms are open in this process.
    #[must_use]
    pub fn open_rooms(&self) -> usize {
        self.registry.lock().len()
    }

    /// Drains every room: appends pending updates, writes Markdown
    /// versions and closes connections so editors reconnect elsewhere.
    pub async fn shutdown(&self) {
        let entries: Vec<RoomEntry> = self.registry.lock().values().cloned().collect();
        let mut acks = Vec::with_capacity(entries.len());
        for entry in entries {
            let (ack, done) = oneshot::channel();
            if entry.ctl.send(Control::Shutdown(ack)).is_ok() {
                acks.push(done);
            }
        }
        for done in acks {
            let _gone = done.await;
        }
    }

    fn room(&self, key: RoomKey) -> RoomEntry {
        let mut rooms = self.registry.lock();
        if let Some(entry) = rooms.get(&key) {
            return entry.clone();
        }
        let generation = self
            .registry
            .next_generation
            .fetch_add(1, Ordering::Relaxed);
        let entry = room::spawn(
            key,
            generation,
            Arc::clone(&self.deps),
            Arc::clone(&self.registry),
        );
        let _previous = rooms.insert(key, entry.clone());
        entry
    }
}

fn room_gone() -> Error {
    Error::new(
        ErrorKind::Unavailable,
        "the collaboration room closed; reconnect",
    )
}
