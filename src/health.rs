use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CriticalComponent {
    TelegramBot,
    NotificationWorker,
}

#[derive(Clone, Debug)]
pub struct HealthState {
    inner: Arc<HealthInner>,
}

#[derive(Debug)]
struct HealthInner {
    telegram_required: bool,
    notification_worker_required: bool,
    telegram_ready: AtomicBool,
    notification_worker_ready: AtomicBool,
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
        Self {
            inner: Arc::new(HealthInner {
                telegram_required,
                notification_worker_required,
                telegram_ready: AtomicBool::new(false),
                notification_worker_ready: AtomicBool::new(false),
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
            && (!self.inner.telegram_required || self.inner.telegram_ready.load(Ordering::Acquire))
            && (!self.inner.notification_worker_required
                || self.inner.notification_worker_ready.load(Ordering::Acquire))
    }

    pub fn begin_shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::Release);
    }
}

impl ComponentHealth {
    fn readiness(&self) -> &AtomicBool {
        match self.component {
            CriticalComponent::TelegramBot => &self.inner.telegram_ready,
            CriticalComponent::NotificationWorker => &self.inner.notification_worker_ready,
        }
    }

    pub fn ready(self) -> ReadyGuard {
        self.readiness().store(true, Ordering::Release);
        ReadyGuard { component: self }
    }
}

impl Drop for ReadyGuard {
    fn drop(&mut self) {
        self.component.readiness().store(false, Ordering::Release);
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
