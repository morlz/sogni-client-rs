use std::{collections::HashSet, pin::Pin, time::Duration};

use futures_util::{Stream, StreamExt, stream::SelectAll};
use tokio::{sync::broadcast, time::Instant};

use crate::{Error, EventReceiver, Project, ProjectStatus, Result};

type Changes = Pin<Box<dyn Stream<Item = ()> + Send>>;

fn changes(mut receiver: EventReceiver) -> Changes {
    Box::pin(async_stream::stream! {
        while let Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) = receiver.recv().await {
            yield ();
        }
    })
}

struct QueueWindow {
    timeout: Duration,
    deadline: Instant,
    was_processing: bool,
}

impl QueueWindow {
    fn new(timeout: Duration, now: Instant) -> Self {
        Self {
            timeout,
            deadline: now + timeout,
            was_processing: false,
        }
    }

    fn observe(&mut self, processing: bool, now: Instant) {
        if !processing && self.was_processing {
            self.deadline = now + self.timeout;
        }
        self.was_processing = processing;
    }
}

fn processing_deadline(project: &Project) -> Option<Instant> {
    // A recovered `active` parent maps to Queued even with a running child.
    // Explicit requeues already suspend each child's processing deadline.
    if project.status().is_finished() || project.status() == ProjectStatus::Pending {
        return None;
    }
    project
        .jobs()
        .iter()
        .filter_map(|job| job.processing_deadline())
        .min()
}

/// Tool timeout bounds queue time. Active jobs have a separate fixed
/// per-attempt runtime budget; status/progress noise extends neither deadline.
/// Finished projects resolve signed URLs under the HTTP request timeout.
pub(super) async fn wait_for_tool_project(
    project: &Project,
    timeout: Duration,
) -> Result<Vec<String>> {
    let mut events = SelectAll::new();
    events.push(changes(project.subscribe()));
    let mut watched = HashSet::new();
    let mut window = QueueWindow::new(timeout, Instant::now());
    let finished = project.wait_for_completion(None);
    tokio::pin!(finished);
    loop {
        for job in project.jobs() {
            if watched.insert(job.id()) {
                events.push(changes(job.subscribe()));
            }
        }
        let runtime_deadline = processing_deadline(project);
        let processing = runtime_deadline.is_some();
        window.observe(processing, Instant::now());
        tokio::select! {
            biased;
            result = &mut finished => return result,
            () = tokio::time::sleep_until(runtime_deadline.unwrap_or(window.deadline)),
                if !project.status().is_finished() => {
                // Recheck current state: a worker may start, finish or be
                // reassigned on the timer's turn. An old attempt cannot act.
                let current_deadline = processing_deadline(project);
                if processing {
                    if current_deadline.is_some_and(|deadline| deadline <= Instant::now()) {
                        return Err(Error::Timeout(format!("project {} exceeded its processing time limit", project.id())));
                    }
                } else if !project.status().is_finished() && current_deadline.is_none() {
                    return Err(Error::Timeout(format!("project {} waited {}s without an active job", project.id(), timeout.as_secs())));
                }
            },
            _ = events.next() => {},
        }
    }
}

#[cfg(test)]
#[path = "wait/tests.rs"]
mod processing_tests;

#[cfg(test)]
#[path = "completed_result_wait_tests.rs"]
mod completed_result_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_window_only_resets_after_processing() {
        let start = Instant::now();
        let mut window = QueueWindow::new(Duration::from_secs(90), start);
        for second in [1, 20, 60, 89] {
            window.observe(false, start + Duration::from_secs(second));
        }
        assert_eq!(window.deadline, start + Duration::from_secs(90));
        window.observe(true, start + Duration::from_secs(80));
        window.observe(true, start + Duration::from_secs(180));
        window.observe(false, start + Duration::from_secs(200));
        assert_eq!(window.deadline, start + Duration::from_secs(290));
        window.observe(false, start + Duration::from_secs(280));
        assert_eq!(window.deadline, start + Duration::from_secs(290));
    }
}
