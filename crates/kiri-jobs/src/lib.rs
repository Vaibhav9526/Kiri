use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub job_id: Uuid,
    pub state: JobState,
    pub completed: u64,
    pub total: Option<u64>,
    pub message: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("illegal job transition from {from:?} to {to:?}")]
pub struct TransitionError {
    pub from: JobState,
    pub to: JobState,
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: Uuid,
    pub state: JobState,
    pub cancel_requested: bool,
}

impl Default for Job {
    fn default() -> Self {
        Self::new()
    }
}
impl Job {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            state: JobState::Queued,
            cancel_requested: false,
        }
    }
    pub fn request_cancel(&mut self) {
        if !self.state.is_terminal() {
            self.cancel_requested = true;
        }
    }
    pub fn transition(&mut self, to: JobState) -> Result<(), TransitionError> {
        let legal = matches!(
            (self.state, to),
            (JobState::Queued, JobState::Running | JobState::Cancelled)
                | (
                    JobState::Running,
                    JobState::Succeeded | JobState::Failed | JobState::Cancelled
                )
        );
        if !legal {
            return Err(TransitionError {
                from: self.state,
                to,
            });
        }
        self.state = to;
        Ok(())
    }
    pub fn progress(
        &self,
        completed: u64,
        total: Option<u64>,
        message: impl Into<String>,
    ) -> JobProgress {
        JobProgress {
            job_id: self.id,
            state: self.state,
            completed,
            total,
            message: message.into(),
            updated_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_legal_transitions() {
        let mut job = Job::new();
        job.transition(JobState::Running).unwrap();
        job.transition(JobState::Succeeded).unwrap();
        assert!(job.state.is_terminal());
    }
    #[test]
    fn rejects_illegal_transitions() {
        let mut job = Job::new();
        assert_eq!(
            job.transition(JobState::Succeeded).unwrap_err(),
            TransitionError {
                from: JobState::Queued,
                to: JobState::Succeeded
            }
        );
        job.transition(JobState::Cancelled).unwrap();
        assert!(job.transition(JobState::Running).is_err());
    }
    #[test]
    fn cancellation_is_cooperative() {
        let mut job = Job::new();
        job.request_cancel();
        assert!(job.cancel_requested);
        job.transition(JobState::Cancelled).unwrap();
    }
}
