use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CriticalComponent {
    TelegramBot = 1,
    NotificationWorker = 2,
}

#[derive(Clone, Debug)]
pub struct HealthState {
    inner: Arc<HealthInner>,
}

#[derive(Debug)]
struct HealthInner {
    required: u8,
    ready: AtomicU8,
    shutting_down: AtomicBool,
}

#[derive(Clone, Debug)]
pub struct ComponentHealth {
    inner: Arc<HealthInner>,
    component: CriticalComponent,
}

#[must_use]
pub struct ReadyGuard {
    component: ComponentHealth,
}

impl HealthState {
    pub fn new(telegram_required: bool, notification_worker_required: bool) -> Self {
        let mut required = 0;
        if telegram_required {
            required |= CriticalComponent::TelegramBot as u8;
        }
        if notification_worker_required {
            required |= CriticalComponent::NotificationWorker as u8;
        }

        Self {
            inner: Arc::new(HealthInner {
                required,
                ready: AtomicU8::new(0),
                shutting_down: AtomicBool::new(false),
            }),
        }
    }

    pub fn component(&self, component: CriticalComponent) -> ComponentHealth {
        ComponentHealth {
            inner: self.inner.clone(),
            component,
        }
    }

    pub fn is_ready_without_database(&self) -> bool {
        !self.inner.shutting_down.load(Ordering::Acquire)
            && self.inner.ready.load(Ordering::Acquire) & self.inner.required == self.inner.required
    }

    pub fn begin_shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::Release);
    }
}

impl ComponentHealth {
    pub fn ready(self) -> ReadyGuard {
        self.inner
            .ready
            .fetch_or(self.component as u8, Ordering::AcqRel);
        ReadyGuard { component: self }
    }
}

impl Drop for ReadyGuard {
    fn drop(&mut self) {
        self.component
            .inner
            .ready
            .fetch_and(!(self.component.component as u8), Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_components_control_readiness() {
        let health = HealthState::new(true, false);
        assert!(!health.is_ready_without_database());

        let guard = health.component(CriticalComponent::TelegramBot).ready();
        assert!(health.is_ready_without_database());

        drop(guard);
        assert!(!health.is_ready_without_database());
    }

    #[test]
    fn shutdown_makes_process_unready() {
        let health = HealthState::new(false, false);
        assert!(health.is_ready_without_database());

        health.begin_shutdown();
        assert!(!health.is_ready_without_database());
    }
}
