//! A counting pool of tokens: how a run bounds the commands it runs at once.
//!
//! `run.gate_jobs` is one pool for the whole process. Every §5 command and
//! Verification row — from any lane's gate, re-check or preflight — holds a
//! token while it runs, so however many lanes gate at once, the machine
//! runs at most that many commands. How a command parallelizes inside
//! itself is its project's business: the pool bounds the commands vitrify
//! runs, and nothing more.
//!
//! Waiters are served by priority (higher first), then in the order they
//! arrived: only the first in line takes a token. A token goes back when
//! it is dropped — on every path out of the code that took it, a panic
//! included.

use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// How often a waiter that can be stopped looks at its stop condition.
const POLL: Duration = Duration::from_millis(50);

pub struct Pool {
    size: usize,
    state: Mutex<State>,
    turn: Condvar,
}

#[derive(Default)]
struct State {
    /// Tokens out now.
    held: usize,
    /// The next arrival's ticket.
    next: u64,
    /// Who waits, in the order they are to be served.
    waiting: BTreeSet<(Reverse<i64>, u64)>,
}

/// One token, held until it is dropped.
pub struct Token<'a> {
    pool: &'a Pool,
}

impl Drop for Token<'_> {
    fn drop(&mut self) {
        let mut s = self.pool.lock();
        s.held = s.held.saturating_sub(1);
        drop(s);
        self.pool.turn.notify_all();
    }
}

impl Pool {
    /// A pool of `size` tokens (at least one), counted in memory.
    pub fn new(size: usize) -> Pool {
        Pool {
            size: size.max(1),
            state: Mutex::new(State::default()),
            turn: Condvar::new(),
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// Tokens out now.
    pub fn held(&self) -> usize {
        self.lock().held
    }

    /// Callers waiting for a token now.
    pub fn waiting(&self) -> usize {
        self.lock().waiting.len()
    }

    /// The state, even if a thread panicked holding the lock: the counts are
    /// only ever changed whole, so what it holds is consistent.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Wait for a token at `priority`.
    pub fn take(&self, priority: i64) -> Token<'_> {
        loop {
            if let Some(token) = self.take_unless(priority, &|| false) {
                return token;
            }
        }
    }

    /// Wait for a token at `priority`, giving up when `stop` says so (an
    /// interrupted run does not queue for work it will not do).
    pub fn take_unless(&self, priority: i64, stop: &dyn Fn() -> bool) -> Option<Token<'_>> {
        let mut s = self.lock();
        let me = (Reverse(priority), s.next);
        s.next += 1;
        s.waiting.insert(me);
        // whoever was first in line may not be any more
        self.turn.notify_all();
        loop {
            let first = s.waiting.first() == Some(&me);
            if first && s.held < self.size {
                s.waiting.remove(&me);
                s.held += 1;
                drop(s);
                // the next in line may find a token free too
                self.turn.notify_all();
                return Some(Token { pool: self });
            }
            if stop() {
                s.waiting.remove(&me);
                drop(s);
                // whoever was behind this waiter may now be first
                self.turn.notify_all();
                return None;
            }
            s = self
                .turn
                .wait_timeout(s, POLL)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Instant;

    fn wait_until(what: &str, cond: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !cond() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The tokens free now, counted by taking them all.
    fn free(pool: &Pool) -> usize {
        let mut taken = Vec::new();
        while let Some(t) = pool.take_unless(0, &|| true) {
            taken.push(t);
        }
        taken.len()
    }

    #[test]
    fn never_more_than_its_size_at_once() {
        let pool = Pool::new(3);
        let now = AtomicUsize::new(0);
        let most = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for t in 0..8 {
                let (pool, now, most) = (&pool, &now, &most);
                s.spawn(move || {
                    for _ in 0..40 {
                        let _token = pool.take(t % 3);
                        let n = now.fetch_add(1, Ordering::SeqCst) + 1;
                        most.fetch_max(n, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_micros(200));
                        now.fetch_sub(1, Ordering::SeqCst);
                    }
                });
            }
        });
        assert_eq!(
            most.load(Ordering::SeqCst),
            3,
            "the pool was filled, never overfilled"
        );
        assert_eq!(pool.held(), 0);
        assert_eq!(pool.waiting(), 0);
        assert_eq!(free(&pool), 3);
    }

    #[test]
    fn served_by_priority_then_arrival() {
        let pool = Pool::new(1);
        let pool = Arc::new(pool);
        let order = Arc::new(Mutex::new(Vec::new()));
        let first = pool.take(0);
        let mut threads = Vec::new();
        // arrivals, in this order: who, priority
        for (n, (who, priority)) in [("a", 0), ("b", 5), ("c", 0), ("d", 5), ("e", -1)]
            .into_iter()
            .enumerate()
        {
            let (pool2, order) = (Arc::clone(&pool), Arc::clone(&order));
            threads.push(std::thread::spawn(move || {
                let _token = pool2.take(priority);
                order.lock().unwrap().push(who);
            }));
            // the next arrives only once this one is queued
            wait_until("a waiter to queue", || pool.waiting() == n + 1);
        }
        drop(first);
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(*order.lock().unwrap(), ["b", "d", "a", "c", "e"]);
    }

    #[test]
    fn a_token_goes_back_when_its_holder_panics() {
        let pool = Pool::new(1);
        let pool = Arc::new(pool);
        let p = Arc::clone(&pool);
        let died = std::thread::spawn(move || {
            let _token = p.take(0);
            panic!("a command runner fell over");
        })
        .join();
        assert!(died.is_err());
        assert_eq!(pool.held(), 0);
        // and the pool still works: its lock is not left poisoned for good
        let _again = pool.take(0);
        assert_eq!(pool.held(), 1);
    }

    #[test]
    fn a_waiter_told_to_stop_leaves_the_queue() {
        let pool = Pool::new(1);
        let held = pool.take(0);
        assert!(pool.take_unless(0, &|| true).is_none());
        assert_eq!(pool.waiting(), 0);
        drop(held);
        assert!(
            pool.take_unless(0, &|| true).is_some(),
            "a free token is taken at once"
        );
        assert_eq!(pool.held(), 0, "and given back when dropped");
    }

    #[test]
    fn a_size_of_zero_is_one() {
        let pool = Pool::new(0);
        assert_eq!(pool.size(), 1);
        let _one = pool.take(0);
        assert!(pool.take_unless(0, &|| true).is_none());
    }

    #[test]
    fn waiters_that_give_up_leave_no_token_behind() {
        let pool = Pool::new(2);
        // one waiter stopped while it waits in line, behind a full pool
        let (a, b) = (pool.take(0), pool.take(0));
        let stop = AtomicBool::new(false);
        std::thread::scope(|s| {
            let waiter = s.spawn(|| {
                pool.take_unless(0, &|| stop.load(Ordering::SeqCst))
                    .is_none()
            });
            wait_until("the waiter to queue", || pool.waiting() == 1);
            stop.store(true, Ordering::SeqCst);
            assert!(waiter.join().unwrap());
        });
        drop((a, b));
        assert_eq!(free(&pool), 2);
        // many, giving up at random while the tokens go round
        std::thread::scope(|s| {
            for t in 0..6u64 {
                let pool = &pool;
                s.spawn(move || {
                    for i in 0..60u64 {
                        let deadline =
                            Instant::now() + Duration::from_micros((t * 7 + i) % 5 * 150);
                        if let Some(_token) = pool.take_unless(0, &|| Instant::now() > deadline) {
                            std::thread::sleep(Duration::from_micros(300));
                        }
                    }
                });
            }
        });
        assert_eq!(pool.waiting(), 0);
        assert_eq!(free(&pool), 2, "every token back");
    }
}
