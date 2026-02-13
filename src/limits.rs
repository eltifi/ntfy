use governor::{Quota, RateLimiter, state::keyed::DefaultKeyedStateStore, clock::DefaultClock};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use dashmap::DashMap;

#[derive(Clone)]
pub struct Limiter {
    // Keyed rate limiter by IP
    visitor_limiter: Arc<RateLimiter<IpAddr, DefaultKeyedStateStore<IpAddr>, DefaultClock>>,
    // Daily bandwidth by IP: (bytes, last_reset)
    bandwidth_tracker: Arc<DashMap<IpAddr, (usize, Instant)>>,
    daily_bandwidth_limit: usize,
}

impl Limiter {
    pub fn new(burst: u32, replenish_ms: u64, daily_bandwidth_limit: usize) -> Self {
        let quota = Quota::with_period(Duration::from_millis(replenish_ms))
            .unwrap()
            .allow_burst(NonZeroU32::new(burst).unwrap());
            
        let visitor_limiter = Arc::new(RateLimiter::keyed(quota));
        
        Self {
            visitor_limiter,
            bandwidth_tracker: Arc::new(DashMap::new()),
            daily_bandwidth_limit,
        }
    }
    
    pub fn check(&self, ip: IpAddr) -> bool {
        self.visitor_limiter.check_key(&ip).is_ok()
    }

    pub fn check_bandwidth(&self, ip: IpAddr, size: usize) -> bool {
        let mut entry = self.bandwidth_tracker.entry(ip).or_insert((0, Instant::now()));
        let (bytes, last_reset) = entry.value_mut();
        
        if last_reset.elapsed() > Duration::from_secs(24 * 3600) {
            *bytes = 0;
            *last_reset = Instant::now();
        }
        
        if *bytes + size > self.daily_bandwidth_limit {
            return false;
        }
        
        *bytes += size;
        true
    }
}
