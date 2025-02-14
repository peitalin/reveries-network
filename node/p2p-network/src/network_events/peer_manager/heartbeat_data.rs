
use std::collections::VecDeque;
use std::time::{Duration, SystemTime};
use tokio::time::Instant;

use crate::behaviour::heartbeat_behaviour::TeeAttestation;


#[derive(Debug, Clone)]
pub struct HeartBeatData {
    pub tee_payload: TeeAttestation,
    pub last_heartbeat: Instant,
    pub last_heartbeat_sys: SystemTime,
    pub size_moving_window: u32,
    pub durations: VecDeque<Duration>,
}

impl HeartBeatData {
    pub fn new(window: u32) -> Self {
        Self {
            tee_payload: TeeAttestation::default(),
            last_heartbeat: Instant::now(),
            last_heartbeat_sys: SystemTime::now(),
            size_moving_window: window,
            durations: VecDeque::with_capacity(window as usize),
        }
    }

    pub fn duration_since_last_heartbeat(&self) -> Duration {
        self.last_heartbeat.elapsed()
    }

    pub fn average_time_between_heartbeats(&self) -> Duration {
        if self.durations.is_empty() {
            Duration::from_secs(0)
        } else {

            let len = u32::try_from(self.durations.len())
                .expect("Size of the window is `u32`, impossible to overflow");

            self.durations.iter()
                .sum::<Duration>()
                .checked_div(len)
                .expect("Length is non-zero because of the check above")
        }
    }

    fn add_new_duration(&mut self, new_duration: Duration) {
        if self.durations.len() == self.size_moving_window as usize {
            self.durations.pop_back();
        }
        self.durations.push_front(new_duration);
    }

    pub fn update(&mut self, tee_payload: TeeAttestation) {
        self.tee_payload = tee_payload;
        let old_heartbeat = self.last_heartbeat;
        self.last_heartbeat = Instant::now();
        self.last_heartbeat_sys = SystemTime::now();
        let new_duration = self.last_heartbeat.saturating_duration_since(old_heartbeat);
        self.add_new_duration(new_duration);
    }
}


#[allow(non_snake_case)]
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn duration_since_last_heartbeat__reads_correctly() {
        let heartbeat_data = HeartBeatData::new(10);
        tokio::time::advance(Duration::from_secs(10)).await;
        assert_eq!(
            heartbeat_data.duration_since_last_heartbeat(),
            Duration::from_secs(10)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn update__works_with_many() {

        let intervals: Vec<u64> = vec![
            1, 67, 77, 111, 230, 36, 633, 11,
            42, 7, 19, 23, 450, 98, 15, 32
        ];
        let mut heartbeat_data = HeartBeatData::new(10);

        for (i, interval) in intervals.clone().into_iter().enumerate() {

            tokio::time::advance(Duration::from_secs(interval)).await;

            heartbeat_data.update(TeeAttestation::default());

            let bottom = if i < 10 { 0 } else { i - 9 };
            let range = &intervals[bottom..=i];

            let expected = range
                .iter()
                .map(|x| Duration::from_secs(*x))
                .sum::<Duration>()
                / range.len() as u32;

            let actual = heartbeat_data.average_time_between_heartbeats();
            assert_eq!(actual, expected);
        }
    }
}