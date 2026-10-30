// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Which node owns a room, by rendezvous (highest random weight) hashing.
//!
//! Every gateway computes the same owner from the same node list, with no
//! coordination. When a node leaves, only its rooms move, spread evenly
//! over the others; when one joins, it takes an even share. The hash is
//! fixed here (FNV-1a then a `splitmix64` finaliser) so it never changes
//! with the Rust release.

use crate::hub::RoomKey;

/// The node among `nodes` that owns `room`, or `None` when the list is
/// empty. Node names must be unique and stable (a pod name, say).
#[must_use]
pub fn owner<'a, N: AsRef<str>>(
    room: &RoomKey,
    nodes: &'a [N],
) -> Option<&'a N> {
    let room_hash = room_hash(room);
    nodes.iter().max_by_key(|node| {
        (
            mix(fnv1a(node.as_ref().as_bytes()) ^ room_hash),
            node.as_ref().to_owned(),
        )
    })
}

fn room_hash(room: &RoomKey) -> u64 {
    let mut bytes = [0_u8; 32];
    let (space, doc) = bytes.split_at_mut(16);
    space.copy_from_slice(&room.space.as_u128().to_be_bytes());
    doc.copy_from_slice(&room.doc.as_u128().to_be_bytes());
    fnv1a(&bytes)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

const fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use vellum_kernel::{DocId, SpaceId};

    use super::*;

    fn rooms(count: usize) -> Vec<RoomKey> {
        let space = SpaceId::generate();
        (0..count)
            .map(|_| RoomKey {
                space,
                doc: DocId::generate(),
            })
            .collect()
    }

    #[test]
    fn spreads_rooms_evenly() {
        let nodes = ["collab-0", "collab-1", "collab-2", "collab-3"];
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for room in rooms(8_000) {
            if let Some(node) = owner(&room, &nodes) {
                *counts.entry(node).or_default() += 1;
            }
        }
        for node in nodes {
            let count = counts.get(node).copied().unwrap_or_default();
            assert!((1_600..=2_400).contains(&count), "{node}: {count}");
        }
    }

    #[test]
    fn only_the_lost_nodes_rooms_move() {
        let before = ["a", "b", "c", "d", "e"];
        let after = ["a", "b", "d", "e"];
        for room in rooms(2_000) {
            let old = owner(&room, &before).copied();
            let new = owner(&room, &after).copied();
            if old != Some("c") {
                assert_eq!(old, new);
            }
        }
    }

    #[test]
    fn no_nodes_no_owner() {
        let none: [&str; 0] = [];
        assert!(rooms(1).iter().all(|room| owner(room, &none).is_none()));
    }
}
