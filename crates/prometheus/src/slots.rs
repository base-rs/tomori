// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The slots a client's requests take, which bound how many are in flight at once, waited for by the callers on their own threads.
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// A client's slots, as many as its concurrency. Background requests may hold all but one, unless there is only one, and never take a slot while another request waits for one.
pub(crate) struct Slots {
    held: Mutex<Held>,
    freed: Condvar,
    limit: usize,
}

#[derive(Default)]
struct Held {
    all: usize,
    background: usize,
    /// Other requests waiting for a slot, which background requests let go first.
    waiting: usize,
}

/// A slot taken, given back as it is dropped.
pub(crate) struct Slot {
    slots: Arc<Slots>,
    background: bool,
}

impl Slots {
    pub fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            held: Mutex::new(Held::default()),
            freed: Condvar::new(),
            limit,
        })
    }

    /// How many slots there are.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// The most slots background requests may hold: all but one, so that one is always left for other requests, unless there is only one.
    pub fn background_limit(&self) -> usize {
        self.limit.saturating_sub(1).max(1)
    }

    fn held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A slot, or `None` if none came free within `wait`. Waiters are not served in order, except that a background request lets every other waiting request go first.
    pub fn take(self: &Arc<Self>, background: bool, wait: Duration) -> Option<Slot> {
        let deadline = Instant::now() + wait;
        let mut held = self.held();
        let mut waiting = false;
        let slot = loop {
            let free = held.all < self.limit
                && (!background
                    || (held.background < self.background_limit() && held.waiting == 0));
            if free {
                held.all += 1;
                held.background += usize::from(background);
                break Some(Slot {
                    slots: self.clone(),
                    background,
                });
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break None;
            }
            if !background && !waiting {
                waiting = true;
                held.waiting += 1;
            }
            held = self
                .freed
                .wait_timeout(held, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        };
        // A request stops waiting only with a slot or with every slot held, so no background request can take one because it stopped.
        held.waiting -= usize::from(waiting);
        slot
    }

    /// How many slots are held, and how many of them by background requests.
    #[cfg(test)]
    pub fn count(&self) -> (usize, usize) {
        let held = self.held();
        (held.all, held.background)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut held = self.slots.held();
        held.all -= 1;
        held.background -= usize::from(self.background);
        drop(held);
        // A background waiter may not take the slot given back, so every waiter is woken.
        self.slots.freed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_requests_leave_one_slot_for_others() {
        let slots = Slots::new(3);
        let none = Duration::ZERO;
        let first = slots.take(true, none).unwrap();
        let _second = slots.take(true, none).unwrap();
        assert!(slots.take(true, none).is_none());
        let other = slots.take(false, none).unwrap();
        assert_eq!(slots.count(), (3, 2));
        assert!(slots.take(false, none).is_none());
        drop(other);
        assert!(slots.take(true, none).is_none());
        drop(first);
        let _third = slots.take(true, none).unwrap();
        assert_eq!(slots.count(), (2, 2));

        // With only one slot, a background request may take it while no other request waits.
        let single = Slots::new(1);
        assert_eq!(single.background_limit(), 1);
        let only = single.take(true, none).unwrap();
        assert!(single.take(false, none).is_none());
        drop(only);
        assert_eq!(single.count(), (0, 0));
    }

    #[test]
    fn a_background_request_lets_a_waiting_request_go_first() {
        let slots = Slots::new(1);
        let held = slots.take(false, Duration::ZERO).unwrap();
        let order = Arc::new(Mutex::new(Vec::new()));
        let waiter = |background: bool| {
            let (slots, order) = (slots.clone(), order.clone());
            std::thread::spawn(move || {
                let slot = slots.take(background, Duration::from_secs(10)).unwrap();
                order.lock().unwrap().push(background);
                // Held a moment, so that a request it went before could not have taken it first.
                std::thread::sleep(Duration::from_millis(50));
                drop(slot);
            })
        };
        let background = waiter(true);
        let other = waiter(false);
        while slots.held().waiting == 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
        drop(held);
        other.join().unwrap();
        background.join().unwrap();
        assert_eq!(*order.lock().unwrap(), [false, true]);
        assert_eq!(slots.held().waiting, 0);
    }

    #[test]
    fn a_waiter_takes_a_slot_given_back_within_its_wait() {
        let slots = Slots::new(1);
        let held = slots.take(false, Duration::ZERO).unwrap();
        let waiting = {
            let slots = slots.clone();
            std::thread::spawn(move || slots.take(false, Duration::from_secs(10)).is_some())
        };
        std::thread::sleep(Duration::from_millis(50));
        drop(held);
        assert!(waiting.join().unwrap());
        let started = Instant::now();
        let _held = slots.take(false, Duration::ZERO).unwrap();
        assert!(slots.take(false, Duration::from_millis(50)).is_none());
        assert!(started.elapsed() >= Duration::from_millis(50));
    }
}
