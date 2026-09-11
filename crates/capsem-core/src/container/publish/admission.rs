//! Setup pacing has no refill task; cancellation releases the FIFO waiter.
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::{sleep, Instant};

struct Credit {
    available: Duration,
    updated: Instant,
}

pub(super) struct SetupRate {
    credit: Mutex<Credit>,
    period: Duration,
    capacity: Duration,
}

impl Default for SetupRate {
    fn default() -> Self {
        Self::new(32, 16)
    }
}

impl SetupRate {
    pub fn new(rate_per_second: u16, burst: u16) -> Self {
        let period = Duration::from_nanos(1_000_000_000u64.div_ceil(u64::from(rate_per_second)));
        let capacity = period * u32::from(burst);
        Self {
            credit: Mutex::new(Credit {
                available: capacity,
                updated: Instant::now(),
            }),
            period,
            capacity,
        }
    }

    pub async fn acquire(&self) {
        let mut credit = self.credit.lock().await;
        loop {
            let now = Instant::now();
            credit.available = credit
                .available
                .saturating_add(now.duration_since(credit.updated))
                .min(self.capacity);
            credit.updated = now;
            if credit.available >= self.period {
                break;
            }
            sleep(
                self.period
                    .checked_sub(credit.available)
                    .expect("credit below one token"),
            )
            .await;
        }
        credit.available -= self.period;
        drop(credit);
    }
}

#[cfg(test)]
mod tests;
