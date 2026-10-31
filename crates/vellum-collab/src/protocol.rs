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
/// When the bytes are not a valid update, or carry a value yrs would trust
/// unchecked: a count larger than the bytes left, a client ID over 53
/// bits, a clock past `u32::MAX` or a too-deep value.
pub fn decode_update(bytes: &[u8]) -> Result<yrs::Update, ReadError> {
    use yrs::updates::decoder::Decode as _;
    contain(|| yrs::Update::decode(&mut UpdateGuard::new(bytes)))
        .ok_or(ReadError::UnexpectedValue)?
}

/// The deepest nesting of arrays and maps accepted in an `Any` value.
///
/// yrs decodes `Any` recursively, so a few megabytes of nested one-element
/// arrays would overflow the stack, which aborts the process rather than
/// panicking. 128 is the limit `serde_json` already applies to the JSON
/// content (embeds, formats) of the same update.
const MAX_ANY_DEPTH: usize = 128;

/// A v1 decoder that refuses every count in an update that claims more
/// entries than the bytes left could hold.
///
/// yrs 0.28 reserves memory for some counts before it reads a single entry
/// (with `try_reserve`, which a 5 or 12 GB request still reaches), so a
/// few bytes once made the server ask for gigabytes. Every entry takes at
/// least one byte, so a count larger than the bytes after it is a lie.
///
/// These are every place where the v1 update grammar puts a count, and how
/// this decoder sees it while yrs decodes (yrs is pinned to 0.28.0 in
/// `Cargo.toml` because the hooks rely on its call order):
///
/// | Count | yrs reserves | Seen as |
/// | --- | --- | --- |
/// | clients in the update | `HashMap::try_reserve` | the `read_var` at offset 0 |
/// | blocks of one client | `VecDeque::try_reserve` | the last `read_var` before `read_client`, which nothing else calls |
/// | strings of JSON content | `Vec::try_reserve` | `read_len` after a block whose content is JSON |
/// | values of `Any` content | `Vec::try_reserve` | `read_len` after a block whose content is `Any` |
/// | entries of an `Any` array or map | `try_reserve` | [`any_fits`] before `read_any` |
/// | clients and ranges of the delete set | nothing (grows as it reads) | — |
///
/// Client IDs are refused over 53 bits wherever an update carries one
/// (block headers, origins, the delete set). Yjs IDs are JavaScript-safe
/// integers; yrs asserts so in debug builds (a panic, which a fuzzer
/// reports as a crash) and silently folds larger ones in release builds,
/// so the guard makes both refuse them.
///
/// Clock arithmetic is bounded the same way. yrs adds each block's length
/// to the client's clock, and the delete set's lengths to their start
/// clocks, unchecked: past `u32::MAX` that panics in debug builds and wraps
/// in release. The guard refuses a delete-set range that would wrap, and a
/// client whose start clock plus its skip, GC and deleted lengths plus the
/// bytes after it could reach past `u32::MAX` (every other block's length,
/// in clock units, is at most the bytes it takes).
///
/// `Any` values are checked by [`any_fits`] before yrs decodes them: their
/// nesting depth ([`MAX_ANY_DEPTH`]) and integer length ([`int_fits`]).
///
/// Skip, GC and deleted-content lengths are clock spans, not stored
/// entries, so they may be far larger than the bytes. Strings and buffers
/// are read whole with `read_exact`, which already fails past the end.
///
/// State vectors and awareness updates inside a frame are decoded by yrs
/// with its own decoder, so [`counts_fit`] checks them before
/// [`decode`] hands the frame over.
struct UpdateGuard<'a> {
    bytes: &'a [u8],
    inner: DecoderV1<'a>,
    /// Where the most recent `read_var` started.
    last_var: usize,
    /// The info byte of the block being read.
    info: u8,
    /// Whether the next `read_var` is a client's start clock.
    clock_next: bool,
    /// Whether the next `read_var` is the length of a skip block.
    skip_next: bool,
    /// The highest clock the current client's blocks could reach.
    clock_bound: u64,
    /// Whether the next `read_var` is a delete-set client ID.
    ds_client: bool,
    /// The start clock of the delete-set range being read.
    ds_clock: u32,
}

impl<'a> UpdateGuard<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            inner: DecoderV1::new(Cursor::new(bytes)),
            last_var: 0,
            info: 0,
            clock_next: false,
            skip_next: false,
            clock_bound: 0,
            ds_client: false,
            ds_clock: 0,
        }
    }

    /// How many bytes are left to read.
    fn remaining(&mut self) -> usize {
        self.inner.read_to_end().map_or(0, <[u8]>::len)
    }

    /// Where the next read starts.
    fn position(&mut self) -> usize {
        self.bytes.len() - self.remaining()
    }

    /// Reads a client ID, refusing one over 53 bits.
    fn read_checked_client(&mut self) -> Result<yrs::ClientID, ReadError> {
        let id: u64 = self.inner.read_var()?;
        if client_fits(id) {
            Ok(yrs::ClientID::new(id))
        } else {
            Err(ReadError::UnexpectedValue)
        }
    }

    /// Reads a block ID the way `DecoderV1` does, refusing a client ID
    /// over 53 bits.
    fn read_checked_id(&mut self) -> Result<yrs::ID, ReadError> {
        let client = self.read_checked_client()?;
        let clock: u32 = self.inner.read_var()?;
        Ok(yrs::ID::new(client, clock))
    }

    /// The number that starts at `at`, already read.
    fn number_at(&self, at: usize) -> Option<u64> {
        self.bytes
            .get(at..)
            .and_then(|from| Cursor::new(from).read_var::<u64>().ok())
    }

    /// Adds `span` clock units to the current client's bound, refusing a
    /// bound past `u32::MAX`.
    fn spend_clock(&mut self, span: Option<u64>) -> Result<(), ReadError> {
        self.clock_bound = span
            .and_then(|span| self.clock_bound.checked_add(span))
            .filter(|&bound| u32::try_from(bound).is_ok())
            .ok_or(ReadError::UnexpectedValue)?;
        Ok(())
    }

    /// Whether the number that starts at `at` (already read) fits in the
    /// bytes left.
    fn count_fits_at(&mut self, at: usize) -> bool {
        let rest = u64::try_from(self.remaining()).unwrap_or(u64::MAX);
        self.bytes
            .get(at..)
            .and_then(|from| Cursor::new(from).read_var::<u64>().ok())
            .is_some_and(|count| count <= rest)
    }
}

impl yrs::encoding::read::Read for UpdateGuard<'_> {
    fn read_exact(&mut self, len: usize) -> Result<&[u8], ReadError> {
        self.inner.read_exact(len)
    }

    fn read_u8(&mut self) -> Result<u8, ReadError> {
        self.inner.read_u8()
    }

    fn read_var<T: VarInt>(&mut self) -> Result<T, ReadError> {
        self.last_var = self.position();
        let value = T::read(self)?;
        if self.last_var == 0 && !self.count_fits_at(0) {
            return Err(ReadError::UnexpectedValue);
        }
        if std::mem::take(&mut self.ds_client)
            && !self.number_at(self.last_var).is_some_and(client_fits)
        {
            return Err(ReadError::UnexpectedValue);
        }
        if std::mem::take(&mut self.clock_next) {
            // Every block after the clock is at least as long in bytes as
            // in clock units, except skip, GC and deleted ones, which are
            // added as they are read.
            self.clock_bound = 0;
            let after = u64::try_from(self.remaining()).ok();
            self.spend_clock(after)?;
            let clock = self.number_at(self.last_var);
            self.spend_clock(clock)?;
        }
        if std::mem::take(&mut self.skip_next) {
            let span = self.number_at(self.last_var);
            self.spend_clock(span)?;
        }
        Ok(value)
    }
}

impl Decoder for UpdateGuard<'_> {
    fn reset_ds_cur_val(&mut self) {
        // yrs calls this right before it reads each delete-set client.
        self.ds_client = true;
        self.inner.reset_ds_cur_val();
    }

    fn read_ds_clock(&mut self) -> Result<u32, ReadError> {
        self.ds_clock = self.inner.read_ds_clock()?;
        Ok(self.ds_clock)
    }

    fn read_ds_len(&mut self) -> Result<u32, ReadError> {
        let len = self.inner.read_ds_len()?;
        // yrs builds the range as `clock..clock + len`: past `u32::MAX` that
        // panics in debug builds and wraps to a backwards range in release.
        if self.ds_clock.checked_add(len).is_none() {
            return Err(ReadError::UnexpectedValue);
        }
        Ok(len)
    }

    fn read_left_id(&mut self) -> Result<yrs::ID, ReadError> {
        self.read_checked_id()
    }

    fn read_right_id(&mut self) -> Result<yrs::ID, ReadError> {
        self.read_checked_id()
    }

    fn read_client(&mut self) -> Result<yrs::ClientID, ReadError> {
        if !self.count_fits_at(self.last_var) {
            return Err(ReadError::UnexpectedValue);
        }
        let client = self.read_checked_client()?;
        self.clock_next = true;
        Ok(client)
    }

    fn read_info(&mut self) -> Result<u8, ReadError> {
        use yrs::block::BLOCK_SKIP_REF_NUMBER;
        let info = self.inner.read_info()?;
        self.info = info;
        self.skip_next = info == BLOCK_SKIP_REF_NUMBER;
        Ok(info)
    }

    fn read_parent_info(&mut self) -> Result<bool, ReadError> {
        self.inner.read_parent_info()
    }

    fn read_type_ref(&mut self) -> Result<u8, ReadError> {
        self.inner.read_type_ref()
    }

    fn read_len(&mut self) -> Result<u32, ReadError> {
        use yrs::block::{
            BLOCK_GC_REF_NUMBER, BLOCK_ITEM_ANY_REF_NUMBER,
            BLOCK_ITEM_DELETED_REF_NUMBER, BLOCK_ITEM_JSON_REF_NUMBER,
        };
        let at = self.position();
        let len = self.inner.read_len()?;
        if self.info == BLOCK_GC_REF_NUMBER {
            self.spend_clock(Some(u64::from(len)))?;
            return Ok(len);
        }
        match self.info & 0b1111 {
            BLOCK_ITEM_JSON_REF_NUMBER | BLOCK_ITEM_ANY_REF_NUMBER
                if !self.count_fits_at(at) =>
            {
                return Err(ReadError::UnexpectedValue);
            }
            BLOCK_ITEM_DELETED_REF_NUMBER => {
                self.spend_clock(Some(u64::from(len)))?;
            }
            _ => {}
        }
        Ok(len)
    }

    fn read_any(&mut self) -> Result<yrs::Any, ReadError> {
        let at = self.position();
        if !self.bytes.get(at..).is_some_and(any_fits) {
            return Err(ReadError::UnexpectedValue);
        }
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

/// Whether the `Any` value that opens `bytes` is well formed, nests at most
/// [`MAX_ANY_DEPTH`] deep and has no array or map claiming more entries
/// than the bytes left. Walks without recursion, mirroring yrs's
/// `Any::decode` tag by tag.
fn any_fits(bytes: &[u8]) -> bool {
    let mut cursor = Cursor::new(bytes);
    // Values still to read in each open array or map, and whether each
    // value there follows a key; the bottom entry is the value itself.
    let mut open: Vec<(u64, bool)> = vec![(1, false)];
    while let Some(level) = open.last_mut() {
        if level.0 == 0 {
            let _done = open.pop();
            continue;
        }
        level.0 -= 1;
        let keyed = level.1;
        if keyed && cursor.read_buf().is_err() {
            return false;
        }
        let ok = match cursor.read_u8() {
            Ok(127 | 126 | 121 | 120) => true,
            Ok(125) => int_fits(&mut cursor),
            Ok(124) => cursor.read_exact(4).is_ok(),
            Ok(123 | 122) => cursor.read_exact(8).is_ok(),
            Ok(119 | 116) => cursor.read_buf().is_ok(),
            Ok(tag @ (118 | 117)) => {
                let Ok(count) = cursor.read_var::<u64>() else {
                    return false;
                };
                let rest = bytes.len().saturating_sub(cursor.next);
                let fits = u64::try_from(rest).is_ok_and(|rest| count <= rest);
                if !fits || open.len() > MAX_ANY_DEPTH {
                    return false;
                }
                open.push((count, tag == 118));
                true
            }
            _ => false,
        };
        if !ok {
            return false;
        }
    }
    true
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

/// Walks a frame the way [`MessageReader`] does and checks every state
/// vector and awareness update in it: the count ([`count_fits`]) and each
/// entry's client ID ([`state_vector_fits`], [`awareness_fits`]).
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
        MSG_AUTH, MSG_AWARENESS, MSG_QUERY_AWARENESS, MSG_SYNC,
        MSG_SYNC_STEP_1, PERMISSION_DENIED,
    };
    Some(match cursor.read_var::<u8>().ok()? {
        MSG_SYNC => {
            let step = cursor.read_var::<u8>().ok()?;
            let payload = cursor.read_buf().ok()?;
            step != MSG_SYNC_STEP_1 || state_vector_fits(payload)
        }
        MSG_AWARENESS => awareness_fits(cursor.read_buf().ok()?),
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

/// Skips an `Any` integer, refusing one longer than nine bytes (61 bits).
///
/// Yjs writes integers of at most 53 bits here (8 bytes). yrs shifts each
/// further byte in unchecked, so a tenth continuation byte overflows the
/// shift and a 64th bit makes the negation overflow: panics in debug
/// builds, garbage in release.
fn int_fits(cursor: &mut Cursor<'_>) -> bool {
    for _ in 0..9 {
        match cursor.read_u8() {
            Ok(byte) if byte & 0x80 == 0 => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

/// Whether a client ID fits the 53 bits Yjs allows.
const fn client_fits(id: u64) -> bool {
    id >> 53 == 0
}

/// Whether a state vector's count fits ([`count_fits`]) and each of its
/// `(client, clock)` entries names a client ID that fits.
fn state_vector_fits(payload: &[u8]) -> bool {
    entries_fit(payload, |cursor| {
        if !client_fits(cursor.read_var::<u64>().ok()?) {
            return Some(false);
        }
        let _clock = cursor.read_var::<u32>().ok()?;
        Some(true)
    })
}

/// Whether an awareness update's count fits ([`count_fits`]) and each of
/// its `(client, clock, state)` entries names a client ID that fits.
fn awareness_fits(payload: &[u8]) -> bool {
    entries_fit(payload, |cursor| {
        if !client_fits(cursor.read_var::<u64>().ok()?) {
            return Some(false);
        }
        let _clock = cursor.read_var::<u32>().ok()?;
        let _state = cursor.read_buf().ok()?;
        Some(true)
    })
}

/// Checks a counted list with `entry`, which says whether one entry is
/// acceptable or `None` when the bytes end. A list that does not parse is
/// left for yrs to reject, so `entry` must judge each value as soon as it
/// reads it: yrs acts on a value (and may panic) before it finds the
/// bytes missing after it.
fn entries_fit(
    payload: &[u8],
    entry: impl Fn(&mut Cursor<'_>) -> Option<bool>,
) -> bool {
    if !count_fits(payload) {
        return false;
    }
    let mut cursor = Cursor::new(payload);
    let Ok(count) = cursor.read_var::<u64>() else {
        return true;
    };
    for _ in 0..count {
        match entry(&mut cursor) {
            Some(true) => {}
            Some(false) => return false,
            None => return true,
        }
    }
    true
}

/// Runs a decoder over untrusted bytes, turning a panic into `None`.
///
/// The checks above refuse every input known to make yrs assert (client
/// IDs over 53 bits); this stays as a backstop, since a panic would end
/// the whole room. Decoding only
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

    /// Every decoder over `bytes` fails, with the update guard's error, and
    /// fast. The guard's error is what shows it caught the count: the
    /// reservations it prevents are never touched, so resident memory
    /// cannot show them, and without it yrs reserves and then reports
    /// `EndOfBuffer` (or the process dies where the reservation fails).
    fn refused(bytes: &[u8]) {
        let started = std::time::Instant::now();
        assert!(
            matches!(decode_update(bytes), Err(ReadError::UnexpectedValue)),
            "{bytes:02x?}"
        );
        assert!(decode(bytes).is_err());
        assert!(decode_peers(bytes).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    /// One client, one block of `Any` content holding `value`, no deletes.
    fn any_update(value: &[u8]) -> Vec<u8> {
        // clients, blocks, client, clock, info (Any, no origins), named
        // parent "a", one value
        let mut update = vec![1, 1, 1, 0, 8, 1, 1, b'a', 1];
        update.extend_from_slice(value);
        update.push(0);
        update
    }

    #[test]
    fn lying_counts_are_rejected_before_allocating() {
        // collab_frames oom-4b9b3b46: five clients, the first claiming
        // 234,880,895 blocks in the four bytes left (5.6 GB).
        refused(&[0x85, 0x00, 0xff, 0xfe, 0xff, 0x6f, 0xdf, 0xff, 0x00, 0x04]);
        // collab_frames oom-9071b3eb: a block of JSON content claiming
        // 536,870,911 strings in the one byte left (12.9 GB).
        refused(&[
            0x02, 0x01, 0x01, 0x32, 0x42, 0x01, 0x01, 0xff, 0xff, 0xff, 0xff,
            0x01, 0xff,
        ]);
        // More clients than bytes.
        refused(&[0xff, 0xff, 0xff, 0x0f, 0]);
        // Any content claiming 2^28 values.
        let mut update = vec![1, 1, 1, 0, 8, 1, 1, b'a'];
        update.extend([0xff, 0xff, 0xff, 0x7f, 126, 0]);
        refused(&update);
        // An Any array, and an Any map nested in one, claiming 2^28 entries.
        refused(&any_update(&[117, 0xff, 0xff, 0xff, 0x7f, 126]));
        refused(&any_update(&[
            117, 1, 118, 0xff, 0xff, 0xff, 0x7f, 1, b'k', 126,
        ]));
    }

    #[test]
    fn oversized_client_ids_are_refused_before_yrs_sees_them() {
        use yrs::updates::decoder::Decode as _;
        // collab_frames crash-d6d83abc (debug assertions on): two clients,
        // the first with no blocks and a 56-bit ID.
        let crash = [
            0x02, 0x00, 0xb1, 0xb1, 0xb1, 0xb1, 0xb1, 0xb1, 0xb1, 0xb1, 0x00,
            0x00,
        ];
        // A block whose right origin names a 64-bit client, and a delete
        // set naming one.
        let mut origin = vec![1, 1, 1, 0, 0x44];
        origin.extend([0xff; 9]);
        origin.extend([0x01, 0, 1, b'x', 0]);
        let mut deletes = vec![0, 1];
        deletes.extend([0xff; 9]);
        deletes.extend([0x01, 1, 0, 1]);
        for update in [&crash[..], &origin, &deletes] {
            // Straight through the guard, without `contain`: yrs would
            // panic (in debug builds) if the ID reached it.
            assert!(
                matches!(
                    yrs::Update::decode(&mut UpdateGuard::new(update)),
                    Err(ReadError::UnexpectedValue)
                ),
                "{update:02x?}"
            );
        }
        // collab_frames crash-7b9bb8fa (overflow checks on): a delete-set
        // range from clock 4,294,967,295 onwards.
        let wraps =
            [0x00, 0x01, 0x07, 0x01, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x01];
        // collab_frames crash-6b37ee07 (overflow checks on): a client at
        // clock 9 whose GC block runs its clock past u32::MAX.
        let crash = [
            0x01, 0x02, 0x01, 0x09, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00,
        ];
        // The same through a skip block, and a deleted item after a high
        // clock.
        let skip = [
            0x01, 0x01, 0x01, 0x09, 0x0a, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x00,
        ];
        let deleted_item = [
            0x01, 0x01, 0x01, 0xf0, 0xff, 0xff, 0xff, 0x0f, 0x01, 0x01, 0x01,
            b'a', 0x20, 0x00,
        ];
        for update in [&wraps[..], &crash, &skip, &deleted_item] {
            assert!(
                matches!(
                    yrs::Update::decode(&mut UpdateGuard::new(update)),
                    Err(ReadError::UnexpectedValue)
                ),
                "{update:02x?}"
            );
        }
        let mut sv = vec![1];
        sv.extend([0xff; 9]);
        sv.extend([0x01, 0]);
        assert!(!state_vector_fits(&sv));
        let mut awareness = vec![1];
        awareness.extend([0xff; 9]);
        awareness.extend([0x01, 1, 2, b'{', b'}']);
        assert!(!awareness_fits(&awareness));
        // collab_frames crash-3a13ef52: an awareness entry whose 64-bit
        // client ID is followed by a truncated state.
        let frame = [
            0x02, 0x02, 0x01, 0x14, 0x02, 0x40, 0x2d, 0x01, 0x02, 0x9c, 0x9c,
            0x9c, 0x9c, 0x9c, 0x9c, 0x9c, 0x9c, 0x9c, 0x9c, 0x61, 0x9c, 0x9c,
            0x02, 0x02,
        ];
        assert!(!counts_fit(&frame));
        let mut sv = vec![1];
        sv.extend([0xff; 9]);
        sv.push(0x01);
        assert!(!state_vector_fits(&sv));
    }

    #[test]
    fn long_any_integers_are_rejected_not_fatal() {
        // Largest that Yjs writes: 2^53 - 1 (eight bytes).
        let mut max = vec![125, 0xbf];
        max.extend([0xff; 6]);
        max.push(0x0f);
        assert!(decode_update(&any_update(&max)).is_ok());
        // collab_frames crash-0981f2ad: a 125 tag followed by eleven bytes
        // with the continuation bit set overflows yrs's shift.
        let mut long = vec![125];
        long.extend([0xff; 10]);
        long.push(0x01);
        assert!(!any_fits(&long));
        refused(&any_update(&long));
    }

    #[test]
    fn deep_any_values_are_rejected_not_fatal() {
        let nested = |depth: usize| {
            let mut value = [117, 1].repeat(depth);
            value.push(126);
            any_update(&value)
        };
        assert!(decode_update(&nested(MAX_ANY_DEPTH)).is_ok());
        // Deep enough to overflow the stack in yrs's recursive decoder.
        refused(&nested(1_000_000));
        assert!(matches!(
            decode_update(&nested(MAX_ANY_DEPTH + 1)),
            Err(ReadError::UnexpectedValue)
        ));
    }

    #[test]
    fn honest_updates_still_decode() {
        use yrs::{Any, Array as _, GetString as _, Text as _, Transact as _};
        let doc = yrs::Doc::new();
        let text = doc.get_or_insert_text("body");
        let list = doc.get_or_insert_array("list");
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "hello");
        text.insert(&mut txn, 5, " world");
        text.remove_range(&mut txn, 0, 1);
        let value = Any::from(vec![
            Any::from(1),
            Any::from(std::collections::HashMap::from([(
                "k".to_owned(),
                Any::from(vec![
                    Any::Null,
                    Any::from("v"),
                    Any::from(vec![0_u8; 3]),
                ]),
            )])),
            Any::from(Vec::<Any>::new()),
        ]);
        let _ = list.insert(&mut txn, 0, value.clone());
        let _ = list.insert(&mut txn, 1, 2.5);
        let bytes = txn.encode_update_v1();
        drop(txn);
        let update =
            decode_update(&bytes).unwrap_or_else(|e| unreachable!("{e}"));
        let copy = yrs::Doc::new();
        let copied = copy.get_or_insert_text("body");
        let copied_list = copy.get_or_insert_array("list");
        let mut txn = copy.transact_mut();
        txn.apply_update(update)
            .unwrap_or_else(|e| unreachable!("{e}"));
        let decoded = copied.get_string(&txn);
        let first = copied_list.get(&txn, 0);
        drop(txn);
        assert_eq!(decoded, "ello world");
        assert_eq!(first, Some(yrs::Out::Any(value)));
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
        let decoded = decode_peers(&encode_peers(&entries))
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(decoded, entries);
    }
}
