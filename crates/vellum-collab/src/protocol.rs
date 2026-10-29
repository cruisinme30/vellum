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
use yrs::encoding::write::Write as _;
use yrs::sync::{Message, MessageReader};
use yrs::updates::decoder::DecoderV1;
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
    contain(|| yrs::Update::decode_v1(bytes)).ok_or(ReadError::UnexpectedValue)?
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
