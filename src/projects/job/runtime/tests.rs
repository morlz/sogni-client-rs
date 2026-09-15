use super::*;

fn processing_job() -> JobSnapshot {
    let mut state = JobSnapshot::pending("FIRST".into(), "PROJECT".into(), 10.0);
    state.status = JobStatus::Processing;
    state
}

#[test]
fn processing_budget_uses_media_network_and_initial_eta() {
    for (network, video, seconds) in [
        (Network::Fast, false, 30 * 60),
        (Network::Fast, true, 90 * 60),
        (Network::Relaxed, false, 2 * 60 * 60),
        (Network::Relaxed, true, 8 * 60 * 60),
    ] {
        assert_eq!(
            runtime_limit(network, video, None),
            Duration::from_secs(seconds)
        );
    }
    assert_eq!(
        runtime_limit(Network::Fast, false, Some(600.0)),
        Duration::from_secs(3600)
    );
    for eta in [Some(-100.0), Some(0.0), Some(f64::NAN)] {
        assert_eq!(
            runtime_limit(Network::Fast, false, eta),
            Duration::from_secs(1800)
        );
    }
    assert_eq!(
        runtime_limit(Network::Fast, false, Some(f64::MAX)),
        Duration::from_secs(12 * 3600)
    );
}

#[test]
fn network_pin_wins_and_unknown_network_uses_generous_floor() {
    let now = Instant::now();
    let state = processing_job();
    for (params, announced, seconds) in [
        (
            json!({"type":"video", "network":"fast"}),
            Some(Network::Relaxed),
            5400,
        ),
        (
            json!({"type":"video", "network":"relaxed"}),
            Some(Network::Fast),
            28800,
        ),
        (json!({"type":"video"}), Some(Network::Fast), 5400),
        (json!({"type":"video"}), None, 28800),
        (json!({"type":"audio"}), None, 7200),
    ] {
        let mut runtime = ProcessingRuntime::new(&params);
        runtime.observe(&state, true, announced, now);
        assert_eq!(
            runtime.deadline(&state),
            Some(now + Duration::from_secs(seconds))
        );
    }
}

#[test]
fn progress_and_recovery_noise_do_not_extend_processing_budget() {
    let start = Instant::now();
    let mut state = processing_job();
    state.eta_seconds = Some(600.0);
    let mut runtime = ProcessingRuntime::new(&json!({"network":"fast"}));
    runtime.observe(&state, true, None, start);
    for seconds in [30, 600, 1799, 3599, 5000] {
        state.step += 1.0;
        state.eta_seconds = Some(20_000.0);
        runtime.observe(
            &state,
            true,
            Some(Network::Relaxed),
            start + Duration::from_secs(seconds),
        );
        assert_eq!(
            runtime.deadline(&state),
            Some(start + Duration::from_secs(3600))
        );
    }
}

#[test]
fn replacement_attempt_invalidates_old_deadline() {
    let start = Instant::now();
    let mut state = processing_job();
    let mut runtime = ProcessingRuntime::new(&json!({"network":"fast"}));
    runtime.observe(&state, true, None, start);
    let old_deadline = runtime.deadline(&state).unwrap();
    state.status = JobStatus::Pending;
    runtime.observe(&state, true, None, start + Duration::from_secs(1700));
    assert!(runtime.deadline(&state).is_none());
    state.id = "REPLACEMENT".into();
    state.status = JobStatus::Processing;
    runtime.observe(&state, true, None, start + Duration::from_secs(1750));
    let new_deadline = runtime.deadline(&state).unwrap();
    assert_eq!(new_deadline, start + Duration::from_secs(3550));
    assert!(new_deadline > old_deadline);
    state.status = JobStatus::Completed;
    assert!(runtime.deadline(&state).is_none());
}

#[test]
fn same_attempt_phase_changes_preserve_the_first_processing_deadline() {
    let start = Instant::now();
    let mut state = processing_job();
    let mut runtime = ProcessingRuntime::new(&json!({"network":"fast"}));
    runtime.observe(&state, true, None, start);
    let deadline = runtime.deadline(&state).unwrap();
    for (seconds, phase) in [(600, JobStatus::Initiating), (1700, JobStatus::Pending)] {
        state.status = phase;
        runtime.observe(&state, true, None, start + Duration::from_secs(seconds));
        assert!(runtime.deadline(&state).is_none());
        state.status = JobStatus::Processing;
        state.eta_seconds = Some(20_000.0);
        runtime.observe(&state, true, None, start + Duration::from_secs(seconds + 1));
        assert_eq!(runtime.deadline(&state), Some(deadline));
    }
}

#[test]
fn requeue_keeps_public_status_and_requires_fresh_processing_activity() {
    let start = Instant::now();
    let mut state = processing_job();
    let mut runtime = ProcessingRuntime::new(&json!({"network":"fast"}));
    runtime.observe(&state, true, None, start);
    runtime.suspend();
    assert_eq!(state.status, JobStatus::Processing);
    assert!(runtime.deadline(&state).is_none());
    state.eta_seconds = Some(10.0);
    runtime.observe(&state, false, None, start + Duration::from_secs(1900));
    assert!(runtime.deadline(&state).is_none());
    state.status = JobStatus::Initiating;
    runtime.observe(&state, true, None, start + Duration::from_secs(1950));
    state.status = JobStatus::Processing;
    runtime.observe(&state, false, None, start + Duration::from_secs(1960));
    assert!(runtime.deadline(&state).is_none());
    runtime.observe(&state, true, None, start + Duration::from_secs(2000));
    assert_eq!(
        runtime.deadline(&state),
        Some(start + Duration::from_secs(3800))
    );
}
