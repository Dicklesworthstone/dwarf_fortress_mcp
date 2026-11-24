//! Bounded foreground wakeups for independently supervised Task monitors.
//! Registrations belong to their supervisor futures and leave no detached work.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::Waker;

#[derive(Default)]
struct Registrations {
    monitors: BTreeMap<String, Option<Waker>>,
    recovering: bool,
    recovery: Option<Waker>,
}

pub(super) struct ProgressSignal {
    registrations: Mutex<Registrations>,
    generation: AtomicU64,
    maximum_monitors: usize,
}

impl ProgressSignal {
    pub(super) fn new(maximum_monitors: usize) -> Self {
        Self {
            registrations: Mutex::new(Registrations::default()),
            generation: AtomicU64::new(0),
            maximum_monitors,
        }
    }

    fn registrations(&self) -> Result<MutexGuard<'_, Registrations>, &'static str> {
        self.registrations
            .lock()
            .map_err(|_| "laboratory Task progress custody poisoned")
    }

    /// The caller supplies the upstream-validated Task ID, after obtaining its
    /// unique application monitor scope. Duplicate recovery cannot steal it.
    pub(super) fn monitor(self: &Arc<Self>, task_id: &str) -> Result<MonitorWake, &'static str> {
        let mut registrations = self.registrations()?;
        if registrations.monitors.contains_key(task_id) {
            return Err("laboratory Task already has a progress registration");
        }
        if registrations.monitors.len() >= self.maximum_monitors {
            return Err("laboratory Task progress registration capacity reached");
        }
        registrations.monitors.insert(task_id.to_owned(), None);
        Ok(MonitorWake {
            signal: Arc::clone(self),
            task_id: task_id.to_owned(),
        })
    }

    pub(super) fn recovery(self: &Arc<Self>) -> Result<RecoveryWake, &'static str> {
        let mut registrations = self.registrations()?;
        if registrations.recovering {
            return Err("laboratory Task service already has a recovery registration");
        }
        registrations.recovering = true;
        Ok(RecoveryWake {
            signal: Arc::clone(self),
        })
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Take bounded owned copies before invoking any waker. A waker may poll,
    /// replace its registration or drop its scope without holding this lock.
    pub(super) fn wake(&self) {
        self.generation.fetch_add(1, Ordering::Release);
        let wakeups = {
            let registrations = match self.registrations.lock() {
                Ok(registrations) => registrations,
                Err(poison) => poison.into_inner(),
            };
            registrations
                .monitors
                .values()
                .filter_map(Clone::clone)
                .chain(registrations.recovery.iter().cloned())
                .collect::<Vec<_>>()
        };
        for waker in wakeups {
            waker.wake();
        }
    }
}

pub(super) struct MonitorWake {
    signal: Arc<ProgressSignal>,
    task_id: String,
}

impl MonitorWake {
    pub(super) fn register(&self, waker: &Waker) -> Result<(), &'static str> {
        let mut registrations = self.signal.registrations()?;
        let current = registrations
            .monitors
            .get_mut(&self.task_id)
            .ok_or("laboratory Task progress registration is unavailable")?;
        if !current.as_ref().is_some_and(|prior| prior.will_wake(waker)) {
            *current = Some(waker.clone());
        }
        Ok(())
    }
}

impl Drop for MonitorWake {
    fn drop(&mut self) {
        let mut registrations = match self.signal.registrations.lock() {
            Ok(registrations) => registrations,
            Err(poison) => poison.into_inner(),
        };
        registrations.monitors.remove(&self.task_id);
    }
}

pub(super) struct RecoveryWake {
    signal: Arc<ProgressSignal>,
}

impl RecoveryWake {
    pub(super) fn register(&self, waker: &Waker) -> Result<(), &'static str> {
        let mut registrations = self.signal.registrations()?;
        if !registrations.recovering {
            return Err("laboratory Task recovery registration is unavailable");
        }
        if !registrations
            .recovery
            .as_ref()
            .is_some_and(|prior| prior.will_wake(waker))
        {
            registrations.recovery = Some(waker.clone());
        }
        Ok(())
    }
}

impl Drop for RecoveryWake {
    fn drop(&mut self) {
        let mut registrations = match self.signal.registrations.lock() {
            Ok(registrations) => registrations,
            Err(poison) => poison.into_inner(),
        };
        registrations.recovery = None;
        registrations.recovering = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::task::Wake;

    #[derive(Default)]
    struct CountWake(AtomicUsize);

    impl Wake for CountWake {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn counter() -> (Arc<CountWake>, Waker) {
        let count = Arc::new(CountWake::default());
        let waker = Waker::from(Arc::clone(&count));
        (count, waker)
    }

    #[test]
    fn foreground_progress_wakes_every_independent_monitor() -> Result<(), &'static str> {
        let signal = Arc::new(ProgressSignal::new(3));
        let first = signal.monitor("first")?;
        let second = signal.monitor("second")?;
        let third = signal.monitor("third")?;
        let (one, one_waker) = counter();
        let (two, two_waker) = counter();
        let (three, three_waker) = counter();
        first.register(&one_waker)?;
        second.register(&two_waker)?;
        third.register(&three_waker)?;
        signal.wake();
        for count in [&one, &two, &three] {
            assert_eq!(count.0.load(Ordering::SeqCst), 1);
        }
        drop(second);
        signal.wake();
        assert_eq!(one.0.load(Ordering::SeqCst), 2);
        assert_eq!(two.0.load(Ordering::SeqCst), 1);
        assert_eq!(three.0.load(Ordering::SeqCst), 2);
        assert_eq!(signal.generation(), 2);
        Ok(())
    }

    #[test]
    fn monitor_capacity_is_released_only_by_its_own_scope() -> Result<(), &'static str> {
        let signal = Arc::new(ProgressSignal::new(2));
        let first = signal.monitor("first")?;
        let second = signal.monitor("second")?;
        assert!(signal.monitor("first").is_err());
        assert!(signal.monitor("third").is_err());
        let (count, waker) = counter();
        second.register(&waker)?;
        drop(first);
        let replacement = signal.monitor("replacement")?;
        assert!(signal.monitor("overflow").is_err());
        signal.wake();
        assert_eq!(count.0.load(Ordering::SeqCst), 1);
        drop(replacement);
        drop(second);
        assert_eq!(Arc::strong_count(&signal), 1);
        Ok(())
    }

    #[test]
    fn a_replaced_waker_cannot_displace_another_monitor() -> Result<(), &'static str> {
        let signal = Arc::new(ProgressSignal::new(2));
        let first = signal.monitor("first")?;
        let second = signal.monitor("second")?;
        let (old, old_waker) = counter();
        let (new, new_waker) = counter();
        let (peer, peer_waker) = counter();
        first.register(&old_waker)?;
        second.register(&peer_waker)?;
        first.register(&new_waker)?;
        signal.wake();
        assert_eq!(old.0.load(Ordering::SeqCst), 0);
        assert_eq!(new.0.load(Ordering::SeqCst), 1);
        assert_eq!(peer.0.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[test]
    fn recovery_has_a_separate_owned_registration_and_progress_generation()
    -> Result<(), &'static str> {
        let signal = Arc::new(ProgressSignal::new(1));
        let monitor = signal.monitor("monitor")?;
        let recovery = signal.recovery()?;
        assert!(signal.recovery().is_err());
        let (ordinary, ordinary_waker) = counter();
        let (recovering, recovery_waker) = counter();
        monitor.register(&ordinary_waker)?;
        let before = signal.generation();
        signal.wake();
        recovery.register(&recovery_waker)?;
        assert_ne!(
            signal.generation(),
            before,
            "progress before registration remains visible"
        );
        signal.wake();
        assert_eq!(ordinary.0.load(Ordering::SeqCst), 2);
        assert_eq!(recovering.0.load(Ordering::SeqCst), 1);
        drop(recovery);
        signal.wake();
        assert_eq!(recovering.0.load(Ordering::SeqCst), 1);
        assert!(signal.recovery().is_ok());
        Ok(())
    }

    #[test]
    fn wake_callbacks_can_reenter_without_holding_progress_custody() -> Result<(), &'static str> {
        struct Reenter {
            signal: Arc<ProgressSignal>,
            observed_unlocked: AtomicUsize,
        }
        impl Wake for Reenter {
            fn wake(self: Arc<Self>) {
                if self.signal.registrations.try_lock().is_ok() {
                    self.observed_unlocked.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
        let signal = Arc::new(ProgressSignal::new(1));
        let monitor = signal.monitor("monitor")?;
        let receiver = Arc::new(Reenter {
            signal: Arc::clone(&signal),
            observed_unlocked: AtomicUsize::new(0),
        });
        monitor.register(&Waker::from(Arc::clone(&receiver)))?;
        signal.wake();
        assert_eq!(receiver.observed_unlocked.load(Ordering::SeqCst), 1);
        drop(monitor);
        assert_eq!(
            Arc::strong_count(&receiver),
            1,
            "the scope releases its retained waker"
        );
        Ok(())
    }
}
