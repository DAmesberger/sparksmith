use super::*;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};
type Fallible = Result<(), Box<dyn std::error::Error>>;

fn request(query: &str) -> Request {
    Request {
        query: query.into(),
        revision: 1,
        folder: None,
        known: Vec::new(),
    }
}

fn found(name: &str) -> traversal::Found {
    traversal::Found {
        candidates: vec![Candidate {
            path: name.into(),
            name: name.into(),
            detail: String::new(),
            known: true,
        }],
        limited: false,
    }
}

#[test]
fn a_blocked_scan_does_not_block_ui_or_start_more_walkers_and_only_latest_query_wins() -> Fallible {
    let ctx = egui::Context::default();
    let mut search = Search::default();
    let (started, running) = mpsc::sync_channel(1);
    let (release, wait) = mpsc::sync_channel(1);
    let main = std::thread::current().id();
    search.refresh_with(request("old"), &ctx, move |_, _| {
        let _ = started.send(std::thread::current().id());
        let _ = wait.recv_timeout(Duration::from_secs(10));
        Some(found("old.pdf"))
    });
    assert_ne!(running.recv_timeout(Duration::from_secs(10))?, main);
    let scans = Arc::new(AtomicUsize::new(0));
    for query in ["superseded", "new"] {
        let scans = scans.clone();
        search.refresh_with(request(query), &ctx, move |_, _| {
            scans.fetch_add(1, Ordering::Relaxed);
            Some(found("new.pdf"))
        });
        assert!(search.result().is_none());
    }
    assert_eq!(
        scans.load(Ordering::Relaxed),
        0,
        "only the blocked scan exists"
    );
    release.send(())?;
    let until = Instant::now() + Duration::from_secs(10);
    while search.result().is_none() && Instant::now() < until {
        let scans = scans.clone();
        search.refresh_with(request("new"), &ctx, move |_, _| {
            scans.fetch_add(1, Ordering::Relaxed);
            Some(found("new.pdf"))
        });
        std::thread::yield_now();
    }
    let matches = search
        .result()
        .ok_or("no result")?
        .as_ref()
        .map_err(String::as_str)?;
    assert_eq!(matches.ranked[0].name, "new.pdf");
    assert_eq!(scans.load(Ordering::Relaxed), 1, "no superseded scan ran");
    Ok(())
}

#[test]
fn revision_folder_attachment_changes_and_dismissal_invalidate_ready_results() -> Fallible {
    let ctx = egui::Context::default();
    let mut search = Search::default();
    let mut input = request("");
    for field in 0..4 {
        let until = Instant::now() + Duration::from_secs(10);
        while search.result().is_none() && Instant::now() < until {
            search.refresh_with(input.clone(), &ctx, |_, _| Some(found("old.pdf")));
            std::thread::yield_now();
        }
        assert!(search.result().is_some());
        match field {
            0 => input.revision += 1,
            1 => input.folder = Some(PathBuf::from("another-folder")),
            2 => input.known.push(PathBuf::from("another.pdf")),
            _ => {
                search.clear();
                assert!(!search.visible());
            }
        }
        if field < 3 {
            search.refresh_with(input.clone(), &ctx, |_, _| Some(found("new.pdf")));
        }
        assert!(
            search.result().is_none(),
            "the old results cannot be selected"
        );
    }
    Ok(())
}
