use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct InferenceQueue {
    semaphore: Arc<Semaphore>,
    counts: Arc<Mutex<Counts>>,
    global_limit: usize,
    per_uid_limit: usize,
}

#[derive(Default)]
struct Counts {
    queued: usize,
    active: usize,
    by_uid: HashMap<u32, usize>,
}

pub struct Reservation {
    queue: InferenceQueue,
    uid: u32,
    phase: Phase,
}

enum Phase {
    Queued,
    Active(OwnedSemaphorePermit),
    Released,
}

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("global local inference queue is full")]
    GlobalFull,
    #[error("per-user local inference queue is full")]
    UserFull,
    #[error("request cancelled while queued")]
    Cancelled,
}

impl InferenceQueue {
    pub fn new(max_active: usize, global_limit: usize, per_uid_limit: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_active)),
            counts: Arc::new(Mutex::new(Counts::default())),
            global_limit,
            per_uid_limit,
        }
    }

    pub fn reserve(&self, uid: u32) -> Result<(Reservation, u32), QueueError> {
        let mut counts = self.counts.lock().expect("queue lock poisoned");
        if counts.queued + counts.active >= self.global_limit {
            return Err(QueueError::GlobalFull);
        }
        let user_count = counts.by_uid.get(&uid).copied().unwrap_or(0);
        if user_count >= self.per_uid_limit {
            return Err(QueueError::UserFull);
        }
        counts.queued += 1;
        *counts.by_uid.entry(uid).or_insert(0) += 1;
        let position = counts.queued as u32;
        Ok((
            Reservation {
                queue: self.clone(),
                uid,
                phase: Phase::Queued,
            },
            position,
        ))
    }
}

impl Reservation {
    pub async fn acquire(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<(), QueueError> {
        let permit = tokio::select! {
            _ = cancellation.cancelled() => return Err(QueueError::Cancelled),
            permit = self.queue.semaphore.clone().acquire_owned() => {
                permit.expect("inference queue semaphore closed")
            }
        };
        let mut counts = self.queue.counts.lock().expect("queue lock poisoned");
        counts.queued = counts.queued.saturating_sub(1);
        counts.active += 1;
        self.phase = Phase::Active(permit);
        Ok(())
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if matches!(self.phase, Phase::Released) {
            return;
        }
        let was_active = matches!(self.phase, Phase::Active(_));
        self.phase = Phase::Released;
        let mut counts = self.queue.counts.lock().expect("queue lock poisoned");
        if was_active {
            counts.active = counts.active.saturating_sub(1);
        } else {
            counts.queued = counts.queued.saturating_sub(1);
        }
        let remove = if let Some(value) = counts.by_uid.get_mut(&self.uid) {
            *value = value.saturating_sub(1);
            *value == 0
        } else {
            false
        };
        if remove {
            counts.by_uid.remove(&self.uid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_per_user_and_global_bounds() {
        let queue = InferenceQueue::new(1, 2, 1);
        let (_first, _) = queue.reserve(1000).unwrap();
        assert!(matches!(queue.reserve(1000), Err(QueueError::UserFull)));
        let (_second, _) = queue.reserve(1001).unwrap();
        assert!(matches!(queue.reserve(1002), Err(QueueError::GlobalFull)));
    }

    #[tokio::test]
    async fn queued_acquire_can_be_cancelled() {
        let queue = InferenceQueue::new(1, 2, 2);
        let (mut first, _) = queue.reserve(1000).unwrap();
        first.acquire(&CancellationToken::new()).await.unwrap();
        let (mut second, _) = queue.reserve(1000).unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(matches!(
            second.acquire(&cancellation).await,
            Err(QueueError::Cancelled)
        ));
    }
}
