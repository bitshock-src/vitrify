//! Injected-clock renewal decision table, the exit-6
//! path with a scripted backend, and (tier-2, `--ignored`) the real-time
//! hang-kill rehearsal.

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use vitrify_cli::run::lease::{
    Clock, LeaseBackend, MonitorParams, MonitorTarget, MonitorVerdict, RenewResult, monitor_worker,
    should_renew,
};
use vitrify_core::model::UtcTime;

fn t(s: &str) -> UtcTime {
    UtcTime::parse(s).unwrap()
}

#[test]
fn renewal_decision_table() {
    let now = t("2026-10-30T12:00:00Z");
    // lease 60m, fraction 0.25 → renew when < 15m remain
    let cases = [
        ("2026-10-30T13:00:00Z", false), // 60m left
        ("2026-10-30T12:20:00Z", false), // 20m left
        ("2026-10-30T12:15:00Z", false), // exactly 15m: not < threshold
        ("2026-10-30T12:14:59Z", true),  // just under
        ("2026-10-30T12:00:00Z", true),  // expired boundary
        ("2026-10-30T11:00:00Z", true),  // long expired
    ];
    for (expires, expected) in cases {
        assert_eq!(
            should_renew(now, t(expires), 60, 0.25),
            expected,
            "expires {expires}"
        );
    }
    // fraction 0 never renews; fraction 1 renews through the whole lease
    assert!(!should_renew(now, t("2026-10-30T12:00:01Z"), 60, 0.0));
    assert!(should_renew(now, t("2026-10-30T12:59:59Z"), 60, 1.0));
}

struct FakeClock(Mutex<i64>);

impl Clock for FakeClock {
    fn now(&self) -> UtcTime {
        // advance 10 simulated minutes per call
        let mut v = self.0.lock().unwrap();
        let now = UtcTime(*v);
        *v += 600;
        now
    }
}

struct ScriptedBackend {
    results: Mutex<Vec<RenewResult>>,
    calls: Mutex<u32>,
}

impl LeaseBackend for ScriptedBackend {
    fn renew(&self, _unit: &str, _holder: &str) -> RenewResult {
        *self.calls.lock().unwrap() += 1;
        self.results
            .lock()
            .unwrap()
            .pop()
            .unwrap_or(RenewResult::LeaseExpired)
    }
}

fn spawn_sleeper() -> std::process::Child {
    let mut cmd = Command::new("/bin/sleep");
    cmd.arg("600")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    cmd.spawn().unwrap()
}

#[test]
fn exit_6_kills_the_group_and_reports_lease_lost() {
    let mut child = spawn_sleeper();
    let pid = child.id();
    let clock = FakeClock(Mutex::new(0));
    let backend = ScriptedBackend {
        // first renew succeeds, second returns exit-6
        results: Mutex::new(vec![
            RenewResult::LeaseExpired,
            RenewResult::Renewed {
                expires: UtcTime(1800),
            },
        ]),
        calls: Mutex::new(0),
    };
    let params = MonitorParams {
        lease_minutes: 60,
        renew_fraction: 0.25,
        worker_timeout_minutes: 100_000,
        tick: Duration::from_millis(1),
    };
    let verdict = monitor_worker(
        MonitorTarget {
            child: &mut child,
            pid,
            unit: "U-001",
            holder: "w1",
            expires: UtcTime(600), // expires soon in simulated time
        },
        &params,
        &clock,
        &backend,
    );
    assert_eq!(verdict, MonitorVerdict::LeaseLost);
    assert!(*backend.calls.lock().unwrap() >= 2);
    // the sleeper is dead
    assert!(child.try_wait().unwrap().is_some());
}

#[test]
fn transient_renewal_failure_keeps_the_worker_running() {
    let mut child = spawn_sleeper();
    let pid = child.id();
    let clock = FakeClock(Mutex::new(0));
    let backend = ScriptedBackend {
        results: Mutex::new(vec![
            RenewResult::LeaseExpired, // eventually ends the monitor
            RenewResult::Failed("lost race".into()),
            RenewResult::Failed("lost race".into()),
        ]),
        calls: Mutex::new(0),
    };
    let params = MonitorParams {
        lease_minutes: 60,
        renew_fraction: 0.25,
        worker_timeout_minutes: 100_000,
        tick: Duration::from_millis(1),
    };
    let verdict = monitor_worker(
        MonitorTarget {
            child: &mut child,
            pid,
            unit: "U-001",
            holder: "w1",
            expires: UtcTime(0),
        },
        &params,
        &clock,
        &backend,
    );
    assert_eq!(verdict, MonitorVerdict::LeaseLost);
    assert_eq!(
        *backend.calls.lock().unwrap(),
        3,
        "failures retried, not fatal"
    );
}

#[test]
fn timeout_kills_the_group() {
    let mut child = spawn_sleeper();
    let pid = child.id();
    let clock = FakeClock(Mutex::new(0));
    let backend = ScriptedBackend {
        results: Mutex::new(vec![RenewResult::Renewed {
            expires: UtcTime(1_000_000),
        }]),
        calls: Mutex::new(0),
    };
    let params = MonitorParams {
        lease_minutes: 60,
        renew_fraction: 0.25,
        worker_timeout_minutes: 30, // simulated clock advances 10m per tick
        tick: Duration::from_millis(1),
    };
    let verdict = monitor_worker(
        MonitorTarget {
            child: &mut child,
            pid,
            unit: "U-001",
            holder: "w1",
            expires: UtcTime(1_000_000),
        },
        &params,
        &clock,
        &backend,
    );
    assert_eq!(verdict, MonitorVerdict::TimedOut);
    assert!(child.try_wait().unwrap().is_some());
}

/// Tier-2 (real time, ~70 s): a hang-mode stub killed at a 1-minute budget;
/// the group is empty afterwards.
#[test]
#[ignore]
fn real_time_hang_killed_at_the_hard_budget() {
    let stub = {
        let vit = std::path::PathBuf::from(env!("CARGO_BIN_EXE_vitrify"));
        let stub = vit.parent().unwrap().join("stub-worker");
        // always build: a no-op when fresh (stale-binary trap otherwise)
        assert!(
            Command::new(env!("CARGO"))
                .args(["build", "-p", "stub-worker", "--quiet"])
                .status()
                .unwrap()
                .success()
        );
        stub
    };
    let mut cmd = Command::new(&stub);
    cmd.arg("prompt")
        .env("STUB_MODE", "hang")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let mut child = cmd.spawn().unwrap();
    let pid = child.id();
    struct Real;
    impl Clock for Real {
        fn now(&self) -> UtcTime {
            UtcTime::now()
        }
    }
    struct NeverExpire;
    impl LeaseBackend for NeverExpire {
        fn renew(&self, _: &str, _: &str) -> RenewResult {
            RenewResult::Renewed {
                expires: UtcTime::now().add_minutes(60),
            }
        }
    }
    let params = MonitorParams {
        lease_minutes: 60,
        renew_fraction: 0.25,
        worker_timeout_minutes: 1,
        tick: Duration::from_secs(1),
    };
    let verdict = monitor_worker(
        MonitorTarget {
            child: &mut child,
            pid,
            unit: "U-001",
            holder: "w1",
            expires: UtcTime::now().add_minutes(60),
        },
        &params,
        &Real,
        &NeverExpire,
    );
    assert_eq!(verdict, MonitorVerdict::TimedOut);
    // group empty
    let probe = Command::new("/bin/kill")
        .args(["-0", "--", &format!("-{pid}")])
        .status()
        .unwrap();
    assert!(!probe.success(), "process group still alive");
}

// ------------------------------------------- leases held for the gate queue

use vitrify_cli::run::lease::Keeper;

struct FixedClock(i64);

impl Clock for FixedClock {
    fn now(&self) -> UtcTime {
        UtcTime(self.0)
    }
}

struct RecordingBackend {
    renewed: Mutex<Vec<String>>,
    answer: fn(&str) -> RenewResult,
}

impl LeaseBackend for RecordingBackend {
    fn renew(&self, unit: &str, _holder: &str) -> RenewResult {
        self.renewed.lock().unwrap().push(unit.to_string());
        (self.answer)(unit)
    }
}

#[test]
fn work_waiting_for_the_gate_keeps_its_lease() {
    // a worker has exited; its unit waits in the integration queue. Nothing
    // but the keeper renews its lease now — and only when it is due.
    let params = MonitorParams {
        lease_minutes: 60,
        renew_fraction: 0.25,
        worker_timeout_minutes: 120,
        tick: Duration::from_millis(1),
    };
    let keeper = Keeper::default();
    let now = 1_000_000;
    keeper.hold("U-001", "w1", UtcTime(now + 10 * 60)); // 10 min left: due
    keeper.hold("U-002", "w2", UtcTime(now + 50 * 60)); // 50 min left: not due
    keeper.hold("U-003", "w3", UtcTime(now + 5 * 60)); // due, but gone
    let backend = RecordingBackend {
        renewed: Mutex::new(Vec::new()),
        answer: |unit| match unit {
            "U-003" => RenewResult::LeaseExpired,
            _ => RenewResult::Renewed {
                expires: UtcTime(1_000_000 + 60 * 60),
            },
        },
    };
    let lost = keeper.renew_due(&params, &FixedClock(now), &backend);
    assert_eq!(lost, vec!["U-003".to_string()]);
    assert_eq!(*backend.renewed.lock().unwrap(), vec!["U-001", "U-003"]);

    // renewed to a full lease: not due again; the lost one is no longer held
    backend.renewed.lock().unwrap().clear();
    assert!(
        keeper
            .renew_due(&params, &FixedClock(now), &backend)
            .is_empty()
    );
    assert!(backend.renewed.lock().unwrap().is_empty());

    // integrated (or rejected): the keeper lets go
    keeper.drop_unit("U-002");
    let later = now + 55 * 60;
    keeper.renew_due(&params, &FixedClock(later), &backend);
    assert_eq!(*backend.renewed.lock().unwrap(), vec!["U-001"]);
}

#[test]
fn a_renewal_that_fails_is_tried_again() {
    let params = MonitorParams {
        lease_minutes: 60,
        renew_fraction: 0.25,
        worker_timeout_minutes: 120,
        tick: Duration::from_millis(1),
    };
    let keeper = Keeper::default();
    keeper.hold("U-001", "w1", UtcTime(600));
    let backend = RecordingBackend {
        renewed: Mutex::new(Vec::new()),
        answer: |_| RenewResult::Failed("lost race".into()),
    };
    for _ in 0..3 {
        assert!(
            keeper
                .renew_due(&params, &FixedClock(0), &backend)
                .is_empty()
        );
    }
    assert_eq!(
        backend.renewed.lock().unwrap().len(),
        3,
        "still held, still tried"
    );
}
