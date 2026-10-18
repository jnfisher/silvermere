//! `SessionId` allocation: a dense `u16` reused from a free list.
//!
//! Under MBBSEmu a connection is known by its channel number (`0..nterms`). [`SessionIds`]
//! hands out the same kind of number: the lowest free id first (as MBBSEmu's
//! `PointerDictionary` does), at most `max` at a time, so the structures that hold "a
//! channel" keep their shape and their bounds.

use std::collections::BinaryHeap;

use silvermere_types::ids::SessionId;

/// Every session slot is taken (`server.max_sessions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("server full: {max} sessions in use")]
pub struct ServerFull {
    /// The cap that was hit.
    pub max: u16,
}

/// The allocator. Ids are `0..max`; a freed id is reused before a fresh one, lowest first.
#[derive(Debug, Clone)]
pub struct SessionIds {
    max: u16,
    /// Ids never handed out are `next..max`.
    next: u16,
    /// Freed ids below `next`, as a min-heap.
    free: BinaryHeap<std::cmp::Reverse<u16>>,
    in_use: usize,
}

impl SessionIds {
    /// An allocator for `max` sessions.
    pub fn new(max: u16) -> Self {
        Self {
            max,
            next: 0,
            free: BinaryHeap::new(),
            in_use: 0,
        }
    }

    /// The cap.
    pub fn max(&self) -> u16 {
        self.max
    }

    /// Ids in use now.
    pub fn in_use(&self) -> usize {
        self.in_use
    }

    /// The lowest free id, or [`ServerFull`].
    pub fn alloc(&mut self) -> Result<SessionId, ServerFull> {
        let id = if let Some(std::cmp::Reverse(id)) = self.free.pop() {
            id
        } else if self.next < self.max {
            let id = self.next;
            self.next += 1;
            id
        } else {
            return Err(ServerFull { max: self.max });
        };
        self.in_use += 1;
        Ok(SessionId(id))
    }

    /// Returns `id` to the free list. Freeing an id that is not in use is a no-op.
    pub fn free(&mut self, id: SessionId) {
        if id.0 >= self.next || self.free.iter().any(|r| r.0 == id.0) {
            return;
        }
        self.free.push(std::cmp::Reverse(id.0));
        self.in_use -= 1;
    }

    /// Whether `id` is allocated.
    pub fn is_allocated(&self, id: SessionId) -> bool {
        id.0 < self.next && !self.free.iter().any(|r| r.0 == id.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowest_free_first() {
        let mut ids = SessionIds::new(4);
        let a: Vec<u16> = (0..4).map(|_| ids.alloc().unwrap().0).collect();
        assert_eq!(a, [0, 1, 2, 3]);
        assert_eq!(ids.alloc(), Err(ServerFull { max: 4 }));
        ids.free(SessionId(2));
        ids.free(SessionId(0));
        ids.free(SessionId(0));
        assert_eq!(ids.in_use(), 2);
        assert_eq!(ids.alloc().unwrap(), SessionId(0));
        assert_eq!(ids.alloc().unwrap(), SessionId(2));
        assert!(ids.alloc().is_err());
    }
}
