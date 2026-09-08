use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::{Context, Wake, Waker},
    thread,
    time::Duration,
};

use super::*;
use crate::SogniClient;

struct ProjectNotificationWake {
    project: Project,
    woke: AtomicBool,
    unlocked: AtomicBool,
}

impl Wake for ProjectNotificationWake {
    fn wake(self: Arc<Self>) {
        let jobs_unlocked = self.project.inner.jobs.try_write().is_some();
        let state_unlocked = self.project.inner.state.try_write().is_some();
        self.unlocked
            .store(jobs_unlocked && state_unlocked, Ordering::SeqCst);
        self.woke.store(true, Ordering::SeqCst);
    }
}

#[test]
fn snapshot_releases_state_before_collecting_jobs() {
    let project = Project::new(
        "PROJECT".into(),
        json!({"type": "image", "numberOfMedia": 1}),
        false,
        Weak::new(),
    );
    let jobs_guard = project.inner.jobs.write();
    let (state_cloned_tx, state_cloned_rx) = mpsc::sync_channel(0);
    let (snapshot_tx, snapshot_rx) = mpsc::sync_channel(1);
    let snapshot_project = project.clone();
    let snapshot_thread = thread::spawn(move || {
        let snapshot = snapshot_project.snapshot_after_state_clone(|| {
            state_cloned_tx.send(()).expect("test receiver is alive");
        });
        snapshot_tx.send(snapshot).expect("test receiver is alive");
    });

    state_cloned_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("snapshot reached job collection");
    let (writer_done_tx, writer_done_rx) = mpsc::sync_channel(1);
    let writer_project = project.clone();
    let writer_thread = thread::spawn(move || {
        writer_project.update(|state| state.queue_position = 7, &["queuePosition"]);
        writer_done_tx.send(()).expect("test receiver is alive");
    });

    writer_done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("a queued state writer was blocked while snapshot waited on the jobs lock");
    drop(jobs_guard);

    let snapshot = snapshot_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("snapshot completes after the jobs lock is released");
    snapshot_thread
        .join()
        .expect("snapshot thread did not panic");
    writer_thread.join().expect("writer thread did not panic");
    assert_eq!(snapshot.queue_position, -1);
    assert_eq!(project.inner.state.read().queue_position, 7);
}

#[tokio::test]
async fn concurrent_job_recovery_reuses_one_child_and_notifies_once() {
    const IMAGE: &str = "BBBBBBBB-BBBB-4BBB-8BBB-BBBBBBBBBBBB";
    let client = SogniClient::builder()
        .api_key("local-fixture")
        .defer_socket_start(true)
        .build()
        .await
        .unwrap();

    let lower_id = IMAGE.to_ascii_lowercase();
    for (first_id, second_id) in [
        (IMAGE.to_owned(), IMAGE.to_owned()),
        (IMAGE.to_owned(), lower_id.clone()),
        (lower_id.clone(), IMAGE.to_owned()),
        (lower_id.clone(), lower_id),
    ] {
        let project = Project::new(
            "PROJECT".into(),
            json!({"type": "image", "numberOfMedia": 1}),
            false,
            Arc::downgrade(&client.projects.inner),
        );
        let mut events = project.subscribe();
        let wake = Arc::new(ProjectNotificationWake {
            project: project.clone(),
            woke: AtomicBool::new(false),
            unlocked: AtomicBool::new(false),
        });
        let waker = Waker::from(Arc::clone(&wake));
        let mut next_event = Box::pin(events.recv());
        assert!(
            next_event
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        let (ready_tx, ready_rx) = mpsc::sync_channel(2);
        let workers = [first_id.clone(), second_id].map(|id| {
            let worker_project = project.clone();
            let ready_tx = ready_tx.clone();
            let (release_tx, release_rx) = mpsc::sync_channel(0);
            let (completed_tx, completed_rx) = mpsc::sync_channel(1);
            let worker = thread::spawn(move || {
                let job = worker_project.ensure_job_before_insert(&id, || {
                    ready_tx.send(()).expect("test receiver is alive");
                    release_rx
                        .recv_timeout(Duration::from_secs(5))
                        .expect("test releases the candidate");
                });
                completed_tx.send(()).expect("test receiver is alive");

                job
            });

            (release_tx, completed_rx, worker)
        });

        // Both callers observed the absent child before either may publish it.
        for _ in 0..2 {
            ready_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("both candidates are ready before insertion");
        }
        let returned = workers.map(|(release, completed, worker)| {
            release.send(()).expect("candidate is waiting");
            completed
                .recv_timeout(Duration::from_secs(5))
                .expect("job insertion completes after release");

            worker.join().expect("job insertion did not panic")
        });
        drop(next_event);

        assert!(wake.woke.load(Ordering::SeqCst));
        assert!(wake.unlocked.load(Ordering::SeqCst));
        assert_eq!(project.jobs().len(), 1);
        assert_eq!(returned[0].id(), IMAGE);
        assert_eq!(returned[1].id(), IMAGE);
        returned[0].update(|state| state.seed = Some(42), &["seed"]);
        assert_eq!(returned[1].snapshot().seed, Some(42));
        assert_eq!(project.jobs()[0].snapshot().seed, Some(42));
        assert_eq!(project.ensure_job(IMAGE).snapshot().seed, Some(42));
        assert_eq!(
            project
                .ensure_job(&IMAGE.to_ascii_lowercase())
                .snapshot()
                .seed,
            Some(42)
        );

        let mut started = Vec::new();
        while let Ok(event) = events.try_recv() {
            if event.name == "jobStarted" {
                started.push(event.data);
            }
        }
        assert_eq!(started, [json!({"jobId": first_id})]);
    }
    client.close().await.unwrap();
}
