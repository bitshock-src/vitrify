//! Orchestrator-side leases: renewal when remaining lease <
//! `renew_fraction` × `lease_minutes`, and the hard worker wall-clock
//! budget. The renewal decision is pure over an injected clock; expiry is
//! advisory — the fencing token protects integration.

use std::process::Child;
use std::time::Duration;

use vitrify_core::model::UtcTime;

use super::worker::kill_group;

/// Pure renewal decision: remaining lease < fraction × lease.
pub fn should_renew(
    now: UtcTime,
    expires: UtcTime,
    lease_minutes: i64,
    renew_fraction: f64,
) -> bool {
    let remaining = expires.0 - now.0;
    (remaining as f64) < renew_fraction * (lease_minutes as f64) * 60.0
}

/// Injected clock (unit-testable without real time).
pub trait Clock: Send + Sync {
    fn now(&self) -> UtcTime;
}

pub struct RealClock;

impl Clock for RealClock {
    fn now(&self) -> UtcTime {
        UtcTime::now()
    }
}

#[derive(Debug, Clone)]
pub enum RenewResult {
    Renewed {
        expires: UtcTime,
    },
    /// exit 6: lease already expired — stop renewing, kill, leave reclaim
    /// to the loop's derive step.
    LeaseExpired,
    /// transient failure (lost race, git hiccup): keep the worker running,
    /// retry on the next tick.
    Failed(String),
}

/// Injected lease operations (the U-011 transaction in production; scripted
/// in tests).
pub trait LeaseBackend: Send + Sync {
    fn renew(&self, unit: &str, holder: &str) -> RenewResult;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonitorVerdict {
    /// The worker exited on its own (exit success recorded by the caller).
    Exited,
    /// Hard budget exceeded: process group killed; the caller releases the
    /// claim and counts the unit failed for the wave.
    TimedOut,
    /// Renewal returned exit 6: group killed; reclaim is the loop's job.
    LeaseLost,
}

#[derive(Debug, Clone)]
pub struct MonitorParams {
    pub lease_minutes: i64,
    pub renew_fraction: f64,
    pub worker_timeout_minutes: i64,
    pub tick: Duration,
}

/// One worker under monitoring.
pub struct MonitorTarget<'a> {
    pub child: &'a mut Child,
    pub pid: u32,
    pub unit: &'a str,
    pub holder: &'a str,
    pub expires: UtcTime,
}

/// Monitor one worker until it exits, times out, or loses its lease.
/// One monitor thread per worker (std threads); 1 s tick in
/// production.
pub fn monitor_worker(
    target: MonitorTarget<'_>,
    params: &MonitorParams,
    clock: &dyn Clock,
    backend: &dyn LeaseBackend,
) -> MonitorVerdict {
    monitor_worker_tracked(target, params, clock, backend).0
}

/// [`monitor_worker`], also returning the lease expiry as last renewed —
/// the orchestrator keeps the lease alive after the worker has exited, until
/// its work is integrated.
pub fn monitor_worker_tracked(
    target: MonitorTarget<'_>,
    params: &MonitorParams,
    clock: &dyn Clock,
    backend: &dyn LeaseBackend,
) -> (MonitorVerdict, UtcTime) {
    let MonitorTarget {
        child,
        pid,
        unit,
        holder,
        mut expires,
    } = target;
    let start = clock.now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return (MonitorVerdict::Exited, expires),
            Ok(None) => {}
            Err(_) => return (MonitorVerdict::Exited, expires),
        }
        let now = clock.now();
        if now.0 - start.0 > params.worker_timeout_minutes * 60 {
            kill_group(pid);
            let _ = child.wait();
            return (MonitorVerdict::TimedOut, expires);
        }
        if should_renew(now, expires, params.lease_minutes, params.renew_fraction) {
            match backend.renew(unit, holder) {
                RenewResult::Renewed { expires: e } => expires = e,
                RenewResult::LeaseExpired => {
                    kill_group(pid);
                    let _ = child.wait();
                    return (MonitorVerdict::LeaseLost, expires);
                }
                RenewResult::Failed(_) => {
                    // advisory: retry next tick; the fencing token protects
                    // integration either way
                }
            }
        }
        std::thread::sleep(params.tick);
    }
}

/// Leases the orchestrator holds for work that is waiting to be integrated:
/// the worker has exited, so nothing else renews them, and a lease that
/// lapsed in the queue would send finished work back to the pool.
#[derive(Default)]
pub struct Keeper {
    held: std::sync::Mutex<Vec<Held>>,
    stop: std::sync::atomic::AtomicBool,
}

struct Held {
    unit: String,
    holder: String,
    expires: UtcTime,
}

impl Keeper {
    /// Keep `unit`'s lease alive from now on.
    pub fn hold(&self, unit: &str, holder: &str, expires: UtcTime) {
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        held.retain(|h| h.unit != unit);
        held.push(Held {
            unit: unit.to_string(),
            holder: holder.to_string(),
            expires,
        });
    }

    /// Stop renewing `unit`'s lease.
    pub fn drop_unit(&self, unit: &str) {
        self.held
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|h| h.unit != unit);
    }

    pub fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// One pass: renew every held lease that is due. Returns the units whose
    /// lease turned out to be gone.
    pub fn renew_due(
        &self,
        params: &MonitorParams,
        clock: &dyn Clock,
        backend: &dyn LeaseBackend,
    ) -> Vec<String> {
        let due: Vec<(String, String)> = {
            let held = self.held.lock().unwrap_or_else(|e| e.into_inner());
            let now = clock.now();
            held.iter()
                .filter(|h| {
                    should_renew(now, h.expires, params.lease_minutes, params.renew_fraction)
                })
                .map(|h| (h.unit.clone(), h.holder.clone()))
                .collect()
        };
        let mut lost = Vec::new();
        for (unit, holder) in due {
            let result = backend.renew(&unit, &holder);
            let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
            match result {
                RenewResult::Renewed { expires } => {
                    if let Some(h) = held.iter_mut().find(|h| h.unit == unit) {
                        h.expires = expires;
                    }
                }
                RenewResult::LeaseExpired => {
                    held.retain(|h| h.unit != unit);
                    lost.push(unit);
                }
                // advisory: retried at the next pass; the fencing token
                // protects integration either way
                RenewResult::Failed(_) => {}
            }
        }
        lost
    }

    /// Renew until [`Keeper::stop`], one pass per `params.tick`.
    pub fn run(&self, params: &MonitorParams, clock: &dyn Clock, backend: &dyn LeaseBackend) {
        while !self.stop.load(std::sync::atomic::Ordering::SeqCst) {
            self.renew_due(params, clock, backend);
            std::thread::sleep(params.tick);
        }
    }
}
