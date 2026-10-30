// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The wire protocol: the standard Yjs sync and awareness messages, plus
//! three Vellum message types in the custom-tag range.
//!
//! A WebSocket binary frame holds one or more messages back to back.
//!
//! | Tag | Message | Direction | Payload |
//! | --- | --- | --- | --- |
//! | 0 | sync (step 1, step 2, update) | both | Yjs |
//! | 1 | awareness | both | Yjs |
//! | 3 | awareness query | client → server | none |
//! | [`MSG_TRACE`] | trace context | client → server | `traceparent` text |
//! | [`MSG_DONE`] | Done pressed / version written | both | none |
//! | [`MSG_PEERS`] | who is connected | server → client | [`PeerEntry`] list |

use vellum_kernel::UserId;
use yrs::encoding::read::Read as _;
use yrs::encoding::read::{Cursor, Error as ReadError};
use yrs::encoding::varint::VarInt;
use yrs::encoding::write::Write as _;
use yrs::sync::{Message, MessageReader};
use yrs::updates::decoder::{Decoder, DecoderV1};
use yrs::updates::encoder::{Encode as _, Encoder as _, EncoderV1};

/// Client → server: the `traceparent` that parents the spans of the
/// connection's following messages.
pub const MSG_TRACE: u8 = 100;

/// Client → server: the editor pressed Done, write a Markdown version now.
/// Server → client: that version is written.
pub const MSG_DONE: u8 = 101;

/// Server → client: who holds each awareness client ID.
///
/// Editors label cursors from this list, never from the self-reported
/// awareness state, so a peer cannot pose as someone else.
pub const MSG_PEERS: u8 = 102;

/// One awareness client of a connected editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerEntry {
    /// The Yjs client ID the editor uses for its awareness state.
    pub client_id: u64,
    /// The signed-in user behind it.
    pub user: UserId,
    /// Whether the user may edit (otherwise they only view).
    pub can_write: bool,
}

/// Encodes messages into one frame.
#[must_use]
pub fn encode(messages: &[Message]) -> Vec<u8> {
    let mut encoder = EncoderV1::new();
    for message in messages {
        message.encode(&mut encoder);
    }
    encoder.to_vec()
}

/// Decodes every message in a frame.
///
/// # Errors
///
/// When the frame is not a valid sequence of messages, including a frame
/// that ends mid-message or carries trailing bytes.
pub fn decode(frame: &[u8]) -> Result<Vec<Message>, ReadError> {
    if !counts_fit(frame) {
        return Err(ReadError::UnexpectedValue);
    }
    let messages = contain(|| {
        let mut decoder = DecoderV1::new(Cursor::new(frame));
        MessageReader::new(&mut decoder).collect::<Result<Vec<_>, _>>()
    })
    .ok_or(ReadError::UnexpectedValue)??;
    // The reader stops quietly at a truncated message; the canonical
    // encoding must account for every byte.
    if encode(&messages).len() == frame.len() {
        Ok(messages)
    } else {
        Err(ReadError::UnexpectedValue)
    }
}

/// Decodes a Yjs v1 update from a peer.
///
/// # Errors
///
/// When the bytes are not a valid update.
pub fn decode_update(bytes: &[u8]) -> Result<yrs::Update, ReadError> {
    use yrs::updates::decoder::Decode as _;
    if !count_fits(bytes) {
        return Err(ReadError::UnexpectedValue);
    }
    contain(|| yrs::Update::decode(&mut BlockCountGuard::new(bytes)))
        .ok_or(ReadError::UnexpectedValue)?
}

/// A v1 decoder that refuses a client's block count larger than the bytes
/// left in the update.
///
/// yrs reads each client's block count and reserves room for that many
/// blocks before it reads a single one, so ten bytes claiming 2^28 blocks
/// once made the server ask for 5.6 GB. Every block takes at least one
/// byte, so a count larger than the bytes after it is a lie. The counts
/// sit between blocks of the previous client, so [`count_fits`] cannot
/// find them without decoding those blocks; this decoder checks each one
/// while yrs decodes.
///
/// yrs reads a client's block count with `read_var` and then calls
/// `read_client`, which nothing else calls, so the last `read_var` before
/// `read_client` is the count.
struct BlockCountGuard<'a> {
    bytes: &'a [u8],
    inner: DecoderV1<'a>,
    /// Where the most recent `read_var` started.
    last_var: usize,
}

impl<'a> BlockCountGuard<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            inner: DecoderV1::new(Cursor::new(bytes)),
            last_var: 0,
        }
    }

    /// How many bytes are left to read.
    fn remaining(&mut self) -> usize {
        self.inner.read_to_end().map_or(0, <[u8]>::len)
    }
}

impl yrs::encoding::read::Read for BlockCountGuard<'_> {
    fn read_exact(&mut self, len: usize) -> Result<&[u8], ReadError> {
        self.inner.read_exact(len)
    }

    fn read_u8(&mut self) -> Result<u8, ReadError> {
        self.inner.read_u8()
    }

    fn read_var<T: VarInt>(&mut self) -> Result<T, ReadError> {
        self.last_var = self.bytes.len() - self.remaining();
        T::read(self)
    }
}

impl Decoder for BlockCountGuard<'_> {
    fn reset_ds_cur_val(&mut self) {
        self.inner.reset_ds_cur_val();
    }

    fn read_ds_clock(&mut self) -> Result<u32, ReadError> {
        self.inner.read_ds_clock()
    }

    fn read_ds_len(&mut self) -> Result<u32, ReadError> {
        self.inner.read_ds_len()
    }

    fn read_left_id(&mut self) -> Result<yrs::ID, ReadError> {
        self.inner.read_left_id()
    }

    fn read_right_id(&mut self) -> Result<yrs::ID, ReadError> {
        self.inner.read_right_id()
    }

    fn read_client(&mut self) -> Result<yrs::ClientID, ReadError> {
        let blocks = self
            .bytes
            .get(self.last_var..)
            .map(|from| Cursor::new(from).read_var::<u64>());
        let rest = u64::try_from(self.remaining()).unwrap_or(u64::MAX);
        match blocks {
            Some(Ok(blocks)) if blocks <= rest => self.inner.read_client(),
            _ => Err(ReadError::UnexpectedValue),
        }
    }

    fn read_info(&mut self) -> Result<u8, ReadError> {
        self.inner.read_info()
    }

    fn read_parent_info(&mut self) -> Result<bool, ReadError> {
        self.inner.read_parent_info()
    }

    fn read_type_ref(&mut self) -> Result<u8, ReadError> {
        self.inner.read_type_ref()
    }

    fn read_len(&mut self) -> Result<u32, ReadError> {
        self.inner.read_len()
    }

    fn read_any(&mut self) -> Result<yrs::Any, ReadError> {
        self.inner.read_any()
    }

    fn read_json(&mut self) -> Result<yrs::Any, ReadError> {
        self.inner.read_json()
    }

    fn read_key(&mut self) -> Result<std::sync::Arc<str>, ReadError> {
        self.inner.read_key()
    }

    fn read_to_end(&mut self) -> Result<&[u8], ReadError> {
        self.inner.read_to_end()
    }
}

/// Whether the count that opens `payload` could be honest.
///
/// yrs sizes a hash table from the first number of an update, a state
/// vector or an awareness update before it reads a single entry, so five
/// bytes claiming 2^27 clients once made the server touch 224 MB. Every
/// entry takes at least one byte, so a count larger than the bytes after
/// it is a lie and the payload is refused before yrs sees it. A payload
/// that does not start with a readable number is left for yrs to reject.
fn count_fits(payload: &[u8]) -> bool {
    let mut cursor = Cursor::new(payload);
    cursor.read_var::<u64>().map_or(true, |count| {
        let rest = payload.len().saturating_sub(cursor.next);
        u64::try_from(rest).is_ok_and(|rest| count <= rest)
    })
}

/// Walks a frame the way [`MessageReader`] does and checks the count of
/// every state vector and awareness update in it with [`count_fits`].
///
/// The walk stops where the frame stops making sense; yrs fails at that
/// same message and never decodes anything after it.
fn counts_fit(frame: &[u8]) -> bool {
    let mut cursor = Cursor::new(frame);
    while cursor.has_content() {
        match next_fits(&mut cursor) {
            Some(true) => {}
            Some(false) => return false,
            None => return true,
        }
    }
    true
}

/// Reads one message for [`counts_fit`]: whether its count fits, or `None`
/// when the bytes are not a message.
fn next_fits(cursor: &mut Cursor<'_>) -> Option<bool> {
    use yrs::sync::protocol::{
        MSG_AUTH, MSG_AWARENESS, MSG_QUERY_AWARENESS, MSG_SYNC, MSG_SYNC_STEP_1, PERMISSION_DENIED,
    };
    Some(match cursor.read_var::<u8>().ok()? {
        MSG_SYNC => {
            let step = cursor.read_var::<u8>().ok()?;
            let payload = cursor.read_buf().ok()?;
            step != MSG_SYNC_STEP_1 || count_fits(payload)
        }
        MSG_AWARENESS => count_fits(cursor.read_buf().ok()?),
        MSG_AUTH => {
            if cursor.read_var::<u8>().ok()? == PERMISSION_DENIED {
                let _reason = cursor.read_buf().ok()?;
            }
            true
        }
        MSG_QUERY_AWARENESS => true,
        _ => {
            let _payload = cursor.read_buf().ok()?;
            true
        }
    })
}

/// Runs a decoder over untrusted bytes, turning a panic into `None`.
///
/// yrs asserts on some malformed input (a client ID over 53 bits panics
/// while decoding), and a panic would end the whole room. Decoding only
/// reads its input, so nothing is left half-changed.
fn contain<T>(decode: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(decode)).ok()
}

/// Encodes a [`MSG_PEERS`] payload.
#[must_use]
pub fn encode_peers(entries: &[PeerEntry]) -> Vec<u8> {
    let mut encoder = EncoderV1::new();
    encoder.write_var(entries.len());
    for entry in entries {
        encoder.write_var(entry.client_id);
        encoder.write_string(&entry.user.to_string());
        encoder.write_var(u8::from(entry.can_write));
    }
    encoder.to_vec()
}

/// Decodes a [`MSG_PEERS`] payload.
///
/// # Errors
///
/// When the payload is malformed.
pub fn decode_peers(payload: &[u8]) -> Result<Vec<PeerEntry>, ReadError> {
    let mut decoder = DecoderV1::new(Cursor::new(payload));
    let len: usize = decoder.read_var()?;
    let mut entries = Vec::with_capacity(len.min(1024));
    for _ in 0..len {
        let client_id: u64 = decoder.read_var()?;
        let user = decoder
            .read_string()?
            .parse()
            .map_err(|_| ReadError::UnexpectedValue)?;
        let can_write = decoder.read_var::<u8>()? == 1;
        entries.push(PeerEntry {
            client_id,
            user,
            can_write,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::sync::SyncMessage;

    #[test]
    fn frames_hold_several_messages() {
        let frame = encode(&[
            Message::Sync(SyncMessage::Update(vec![0, 0])),
            Message::Custom(MSG_DONE, Vec::new()),
            Message::AwarenessQuery,
        ]);
        let messages = decode(&frame).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages.get(1),
            Some(&Message::Custom(MSG_DONE, Vec::new()))
        );
    }

    #[test]
    fn truncated_and_padded_frames_are_rejected() {
        let frame = encode(&[Message::Sync(SyncMessage::Update(vec![0, 0]))]);
        let truncated = frame.get(..frame.len() - 1).unwrap_or_default();
        assert!(decode(truncated).is_err());
        let mut padded = frame;
        padded.push(0);
        assert!(decode(&padded).is_err());
        assert!(decode(&[0, 2, 200, 1, 2, 3]).is_err());
    }

    #[test]
    fn oversized_client_ids_are_rejected_not_fatal() {
        // Awareness with one client whose ID needs all 64 bits.
        let mut inner = vec![1];
        inner.extend([0xff; 9]);
        inner.push(0x01);
        inner.extend([1, 2, b'{', b'}']);
        let mut frame = vec![1, u8::try_from(inner.len()).unwrap_or(0)];
        frame.extend(inner);
        assert!(decode(&frame).is_err());
        // A sync update naming such a client.
        let mut update = vec![1, 1];
        update.extend([0xff; 9]);
        update.extend([0x01, 0]);
        assert!(decode_update(&update).is_err());
    }

    #[test]
    fn lying_block_counts_are_rejected_before_allocating() {
        // Fuzzer reproducer (collab_frames oom-4b9b3b46): five clients, the
        // first claiming 234,880,895 blocks in the four bytes left. yrs
        // reserved 5.6 GB for them before reading one.
        let update = [0x85, 0x00, 0xff, 0xfe, 0xff, 0x6f, 0xdf, 0xff, 0x00, 0x04];
        let started = std::time::Instant::now();
        assert!(decode_update(&update).is_err());
        assert!(decode(&update).is_err());
        assert!(decode_peers(&update).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn honest_updates_still_decode() {
        use yrs::{GetString as _, Text as _, Transact as _};
        let doc = yrs::Doc::new();
        let text = doc.get_or_insert_text("body");
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "hello");
        text.insert(&mut txn, 5, " world");
        text.remove_range(&mut txn, 0, 1);
        let bytes = txn.encode_update_v1();
        drop(txn);
        let update = decode_update(&bytes).unwrap_or_else(|e| unreachable!("{e}"));
        let copy = yrs::Doc::new();
        let copied = copy.get_or_insert_text("body");
        let mut txn = copy.transact_mut();
        txn.apply_update(update)
            .unwrap_or_else(|e| unreachable!("{e}"));
        let decoded = copied.get_string(&txn);
        drop(txn);
        assert_eq!(decoded, "ello world");
    }

    #[test]
    fn peers_round_trip() {
        let entries = vec![
            PeerEntry {
                client_id: 42,
                user: UserId::generate(),
                can_write: true,
            },
            PeerEntry {
                client_id: u64::from(u32::MAX) + 7,
                user: UserId::generate(),
                can_write: false,
            },
        ];
        let decoded = decode_peers(&encode_peers(&entries)).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(decoded, entries);
    }
}
