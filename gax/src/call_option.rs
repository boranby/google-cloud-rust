use rand::Rng;
use std::time::Duration;
use tonic::{Code, Status};

/// Retryer is used by Invoke to determine retry behavior.
pub trait Retryer {
    fn retry(&mut self, status: &Status) -> Option<Duration>;
}

/// Backoff implements exponential backoff.
/// The wait time between retries is a random value between 0 and the "retry envelope".
/// The envelope starts at Initial and increases by the factor of Multiplier every retry,
/// but is capped at Max.
#[derive(Clone)]
pub struct Backoff {
    pub initial: Duration,
    pub max: Duration,
    pub multiplier: f64,
    pub cur: Duration,
}

impl Backoff {
    fn duration(&mut self) -> Duration {
        // Select a duration between 1ns and the current max. It might seem
        // counterintuitive to have so much jitter, but
        // https://www.awsarchitectureblog.com/2015/03/backoff.html argues that
        // that is the best strategy.
        let mut rng = rand::thread_rng();
        let cur_val = self.cur.as_nanos();
        let d = Duration::from_nanos((1 + rng.gen_range(0..cur_val)) as u64);
        self.cur = Duration::from_nanos((cur_val as f64 * self.multiplier) as u64);
        if self.cur > self.max {
            self.cur = self.max;
        }
        return d;
    }
}

/// CallSettings allow fine-grained control over how calls are made.
#[derive(Clone)]
pub struct BackoffRetryer {
    pub backoff: Backoff, // supports backoff retry only
    pub codes: Vec<tonic::Code>,
    pub check_session_not_found: bool,
}

impl Default for Backoff {
    fn default() -> Self {
        Backoff {
            initial: Duration::from_micros(250),
            max: Duration::from_micros(32000),
            multiplier: 1.30,
            cur: Duration::from_nanos(0),
        }
    }
}

#[derive(Clone)]
pub struct CallSettings {
    pub retryer: BackoffRetryer,
}

impl Retryer for BackoffRetryer {
    fn retry(&mut self, status: &Status) -> Option<Duration> {
        let code = status.code();
        if code == Code::Internal
            && !status.message().contains("stream terminated by RST_STREAM")
            && !status
                .message()
                .contains("HTTP/2 error code: INTERNAL_ERROR")
            && !status
                .message()
                .contains("Connection closed with unknown cause")
            && !status
                .message()
                .contains("Received unexpected EOS on DATA frame from server")
        {
            return None;
        }

        for candidate in self.codes.iter() {
            if *candidate == code {
                log::debug!("retry {} {}", status.code(), status.message());
                return Some(self.backoff.duration());
            }
        }

        if self.check_session_not_found {
            if status.message().contains("Session not found:") {
                log::debug!("retry by session not found");
                return Some(self.backoff.duration());
            }
        }
        return None;
    }
}
