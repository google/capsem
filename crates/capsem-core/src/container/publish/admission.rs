//! Setup pacing has no refill task; cancellation releases the FIFO waiter.
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::{sleep, Instant};

const PERIOD: Duration = Duration::from_micros(31_250);
const CAPACITY: Duration = Duration::from_millis(500);

struct Credit {
    available: Duration,
    updated: Instant,
}

pub(super) struct SetupRate(Mutex<Credit>);

impl Default for SetupRate {
    fn default() -> Self {
        Self(Mutex::new(Credit {
            available: CAPACITY,
            updated: Instant::now(),
        }))
    }
}

impl SetupRate {
    pub async fn acquire(&self) {
        let mut credit = self.0.lock().await;
        loop {
            let now = Instant::now();
            credit.available = credit
                .available
                .saturating_add(now.duration_since(credit.updated))
                .min(CAPACITY);
            credit.updated = now;
            if credit.available >= PERIOD {
                break;
            }
            sleep(PERIOD.checked_sub(credit.available).expect("credit below one token")).await;
        }
        credit.available -= PERIOD;
        drop(credit);
    }
}

#[cfg(test)]
mod tests;
