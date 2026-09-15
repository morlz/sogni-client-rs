use super::*;
use tokio::time::Instant;

pub(super) struct ProcessingRuntime {
    pinned_network: Option<Network>,
    video: bool,
    attempt: Option<(String, Instant)>,
    suspended: bool,
}

impl ProcessingRuntime {
    pub(super) fn new(params: &Value) -> Self {
        Self {
            pinned_network: match params.get("network").and_then(Value::as_str) {
                Some("fast") => Some(Network::Fast),
                Some("relaxed") => Some(Network::Relaxed),
                _ => None,
            },
            video: params.get("type").and_then(Value::as_str) == Some("video"),
            attempt: None,
            suspended: false,
        }
    }

    pub(super) fn observe(
        &mut self,
        state: &JobSnapshot,
        processing_update: bool,
        announced_network: Option<Network>,
        now: Instant,
    ) {
        if state.status.is_finished() {
            self.attempt = None;
            self.suspended = false;
            return;
        }
        // Recovery and late model-loading frames can temporarily regress the
        // public phase without starting another worker attempt. Keep its first
        // deadline until an explicit retry/requeue, replacement, or terminal state.
        if state.status != JobStatus::Processing {
            return;
        }
        if self.attempt.as_ref().is_some_and(|(id, _)| *id == state.id) {
            return;
        }
        // A whole-project requeue does not change the public child status.
        // Only fresh processing activity may arm the replacement attempt;
        // delayed ETA/preview updates still belong to the abandoned worker.
        if self.suspended && !processing_update {
            return;
        }
        self.suspended = false;
        let network = self
            .pinned_network
            .or(announced_network)
            .unwrap_or(Network::Relaxed);
        self.attempt = Some((
            state.id.clone(),
            now + runtime_limit(network, self.video, state.eta_seconds),
        ));
    }

    pub(super) fn suspend(&mut self) {
        self.attempt = None;
        self.suspended = true;
    }

    pub(super) fn deadline(&self, state: &JobSnapshot) -> Option<Instant> {
        self.attempt.as_ref().and_then(|(id, deadline)| {
            (state.status == JobStatus::Processing && *id == state.id).then_some(*deadline)
        })
    }
}

fn runtime_limit(network: Network, video: bool, eta_seconds: Option<f64>) -> Duration {
    // Match the upstream per-attempt backstop. The initial worker ETA can
    // lengthen a budget, but subsequent progress never moves its deadline.
    let floor = match (network, video) {
        (Network::Fast, false) => 30 * 60,
        (Network::Fast, true) => 90 * 60,
        (Network::Relaxed, false) => 2 * 60 * 60,
        (Network::Relaxed, true) => 8 * 60 * 60,
    };
    let eta_budget = eta_seconds
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .map_or(0.0, |seconds| seconds * 6.0);
    Duration::from_secs_f64(eta_budget.max(f64::from(floor)).min(12.0 * 60.0 * 60.0))
}

#[cfg(test)]
mod tests;
