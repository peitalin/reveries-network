
use std::collections::VecDeque;
use std::time::{Duration, SystemTime};
use tokio::time::Instant;

use crate::event_loop::heartbeat::heartbeat_handler::TeeAttestation;


#[derive(Debug, Clone)]
pub struct HeartBeatData {
    pub heartbeat_payload: TeeAttestation,
    pub last_heartbeat: Instant,
    pub last_heartbeat_sys: SystemTime,
    pub size_moving_window: u32,
    pub durations: VecDeque<Duration>,
}

impl HeartBeatData {
    pub fn new(window: u32) -> Self {
        Self {
            heartbeat_payload: TeeAttestation {
                tee_attestation: None,
                tee_attestation_bytes: None,
                block_height: 1
            },
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

    pub fn update(&mut self, heartbeat_payload: TeeAttestation) {
        self.heartbeat_payload = heartbeat_payload;
        let old_heartbeat = self.last_heartbeat;
        self.last_heartbeat = Instant::now();
        self.last_heartbeat_sys = SystemTime::now();
        let new_duration = self.last_heartbeat.saturating_duration_since(old_heartbeat);
        self.add_new_duration(new_duration);
    }

    pub fn increment_block_height(&mut self) {
        self.update(TeeAttestation {
            tee_attestation: self.heartbeat_payload.tee_attestation.clone(),
            tee_attestation_bytes: self.heartbeat_payload.tee_attestation_bytes.clone(),
            block_height: self.heartbeat_payload.block_height + 1
        });
    }

}


// #[allow(non_snake_case)]
// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[tokio::test(start_paused = true)]
//     async fn duration_since_last_heartbeat__reads_correctly() {
//         let heartbeat_data = HeartBeatData::new(10);
//         tokio::time::advance(Duration::from_secs(10)).await;
//         assert_eq!(
//             heartbeat_data.duration_since_last_heartbeat(),
//             Duration::from_secs(10)
//         );
//     }

//     #[tokio::test(start_paused = true)]
//     async fn update__works_with_many() {
//         let intervals: Vec<u64> =
//             vec![5, 40, 19, 400, 23, 36, 33, 22, 11, 10, 9, 8, 72, 16, 5, 4];
//         let mut heartbeat_data = HeartBeatData::new(10);
//         for (i, interval) in intervals.clone().into_iter().enumerate() {
//             tokio::time::advance(Duration::from_secs(interval)).await;
//             heartbeat_data.update(1.into());
//             let bottom = if i < 10 { 0 } else { i - 9 };
//             let range = &intervals[bottom..=i];
//             let expected = range
//                 .iter()
//                 .map(|x| Duration::from_secs(*x))
//                 .sum::<Duration>()
//                 / range.len() as u32;
//             let actual = heartbeat_data.average_time_between_heartbeats();
//             assert_eq!(actual, expected);
//         }
//     }
// }