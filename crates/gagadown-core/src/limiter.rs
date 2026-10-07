use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Global token bucket; `rate == 0` disables it with a single atomic load.
pub struct Limiter {
    rate: AtomicU64,
    state: Mutex<(f64, Instant)>,
}

impl Limiter {
    pub fn new(rate: u64) -> Self {
        Self { rate: AtomicU64::new(rate), state: Mutex::new((0.0, Instant::now())) }
    }

    pub fn set_rate(&self, rate: u64) {
        self.rate.store(rate, Ordering::Relaxed);
    }

    pub fn rate(&self) -> u64 {
        self.rate.load(Ordering::Relaxed)
    }

    pub async fn acquire(&self, n: usize) {
        loop {
            let rate = self.rate.load(Ordering::Relaxed);
            if rate == 0 {
                return;
            }
            let wait = {
                let mut st = self.state.lock();
                let now = Instant::now();
                let burst = (rate as f64 / 4.0).max(64.0 * 1024.0);
                st.0 = (st.0 + now.duration_since(st.1).as_secs_f64() * rate as f64).min(burst);
                st.1 = now;
                if st.0 >= n as f64 || st.0 >= burst * 0.99 {
                    st.0 -= n as f64;
                    return;
                }
                Duration::from_secs_f64(((n as f64 - st.0) / rate as f64).min(0.5))
            };
            tokio::time::sleep(wait).await;
        }
    }
}
