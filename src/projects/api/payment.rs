use tokio::{
    sync::{broadcast, watch},
    time::Instant as TokioInstant,
};

use super::*;
use crate::auth::RequestSession;
use crate::projects::payment::normalize_reply;

const SWITCH_TIMEOUT: Duration = Duration::from_secs(45);
type SwitchKey = (u64, String);
pub(super) type PendingSwitches =
    HashMap<SwitchKey, watch::Receiver<Option<PayAsYouGoSwitchResult>>>;

impl ProjectsApi {
    /// Accept the current server offer to move a waiting plan project to
    /// Premium Spark. Concurrent calls for the same project share the result.
    /// After a reconnect the same request is sent at most once more. The
    /// service makes repeats idempotent. An unknown outcome may have charged
    /// the project; check Spark history or the result if a repeat stays unknown.
    pub async fn switch_to_premium_spark(
        &self,
        project_id: &str,
        timeout: Option<Duration>,
    ) -> Result<PayAsYouGoSwitchResult> {
        self.switch_project_payment(
            project_id,
            timeout,
            self.inner.client.rest.request_session(),
        )
        .await
    }

    pub(in crate::projects) async fn switch_project_payment(
        &self,
        project_id: &str,
        timeout: Option<Duration>,
        session: RequestSession,
    ) -> Result<PayAsYouGoSwitchResult> {
        if project_id.trim().is_empty() || project_id.encode_utf16().count() > 128 {
            return Err(Error::InvalidInput(
                "a project id of up to 128 characters is required".into(),
            ));
        }
        let timeout = timeout
            .filter(|timeout| !timeout.is_zero())
            .unwrap_or(SWITCH_TIMEOUT);
        // Tokio's time range is platform-dependent. Refuse durations that
        // cannot form a deadline, even if supplied by a caller.
        if TokioInstant::now().checked_add(timeout).is_none() {
            return Err(Error::InvalidInput(
                "payment switch timeout is too large".into(),
            ));
        }
        let key = (session.id(), project_id.to_uppercase());
        let mut result = {
            let mut switches = self.inner.payment_switches.lock();
            if let Some(result) = switches.get(&key) {
                result.clone()
            } else {
                let (send_result, result) = watch::channel(None);
                switches.insert(key.clone(), result.clone());
                let api = self.clone();
                let project_id = project_id.to_owned();
                // A caller dropping its future does not abort a shared paid request.
                tokio::spawn(async move {
                    let outcome = api
                        .await_payment_switch(&project_id, timeout, session)
                        .await;
                    if outcome.clears_offer() {
                        let project = api
                            .inner
                            .projects
                            .read()
                            .get(&key.1)
                            .filter(|p| p.auth_session() == key.0)
                            .cloned();
                        if let Some(project) = project {
                            project.clear_pay_as_you_go();
                        }
                    }
                    let mut switches = api.inner.payment_switches.lock();
                    send_result.send_replace(Some(outcome));
                    switches.remove(&key);
                });
                result
            }
        };
        Ok(result
            .wait_for(Option::is_some)
            .await
            .ok()
            .and_then(|result| result.clone())
            .unwrap_or_else(|| unknown(project_id, PayAsYouGoUnknownReason::SessionChanged)))
    }

    async fn await_payment_switch(
        &self,
        project_id: &str,
        timeout: Duration,
        mut session: RequestSession,
    ) -> PayAsYouGoSwitchResult {
        let client = &self.inner.client;
        let mut events = client.subscribe_scoped();
        let send = || client.send_payment_switch(project_id, session.id());
        let (sent_on, sent_after) = match session.run(async { Ok(send().await) }).await {
            Ok(Some(receipt)) => receipt,
            Ok(None) => {
                return PayAsYouGoSwitchResult::new(
                    project_id,
                    PayAsYouGoSwitchOutcome::Refused {
                        reason: PayAsYouGoRefusalReason::NotSent,
                    },
                );
            }
            Err(_) => return unknown(project_id, PayAsYouGoUnknownReason::SessionChanged),
        };
        let mut deadline = TokioInstant::now()
            .checked_add(timeout)
            .unwrap_or_else(TokioInstant::now);
        let mut resent = false;
        loop {
            let event = tokio::select! {
                biased;
                () = session.changed() => return unknown(project_id, PayAsYouGoUnknownReason::SessionChanged),
                () = tokio::time::sleep_until(deadline) => return unknown(project_id, PayAsYouGoUnknownReason::Timeout),
                event = events.recv() => event,
            };
            match event {
                Ok(event) if event.session == Some(session.id()) && event.sequence > sent_after => {
                    if event.event.name == "projectPaymentSwitch"
                        && event
                            .event
                            .data
                            .get("jobID")
                            .and_then(Value::as_str)
                            .is_some_and(|id| id.to_uppercase() == project_id.to_uppercase())
                    {
                        return normalize_reply(project_id, &event.event.data);
                    }
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return unknown(project_id, PayAsYouGoUnknownReason::SessionChanged);
                }
                _ => {}
            }
            if !resent
                && client.is_socket_authenticated()
                && client
                    .socket_generation()
                    .is_some_and(|generation| generation != sent_on)
            {
                resent = true;
                if session
                    .run(async { Ok(client.send_payment_switch(project_id, session.id()).await) })
                    .await
                    .is_err()
                {
                    return unknown(project_id, PayAsYouGoUnknownReason::SessionChanged);
                }
                deadline = TokioInstant::now()
                    .checked_add(timeout)
                    .unwrap_or_else(TokioInstant::now);
            }
        }
    }
}

fn unknown(project_id: &str, reason: PayAsYouGoUnknownReason) -> PayAsYouGoSwitchResult {
    PayAsYouGoSwitchResult::new(project_id, PayAsYouGoSwitchOutcome::Unknown { reason })
}

#[cfg(test)]
mod tests;
