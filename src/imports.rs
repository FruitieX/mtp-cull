use crate::mtp_worker::{ImportPaths, MtpMediaKind, RemoteAsset, RemoteShot, plan_import};
use crate::review;
use crate::review::{Asset, Kind};
use crate::safe_copy::{CopyOutcome, copy_reader_with_progress};
use color_eyre::eyre::{Result, bail, eyre};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
pub enum ImportUpdate {
    Progress { name: String, done: u64, total: u64 },
    Finished(std::result::Result<(usize, usize), String>),
}
pub struct ImportOperation {
    pub receiver: mpsc::Receiver<ImportUpdate>,
    pub cancel: Arc<AtomicBool>,
    pub text: String,
    pub progress: f32,
}
pub fn start(assets: Vec<Asset>, destinations: ImportPaths) -> ImportOperation {
    let cancel = Arc::new(AtomicBool::new(false));
    let thread_cancel = cancel.clone();
    let (sender, receiver) = mpsc::channel();
    let operation = ImportOperation {
        receiver,
        cancel,
        text: "Planning import".into(),
        progress: 0.0,
    };
    std::thread::spawn(move || {
        let result: Result<(usize, usize)> = (|| {
            let shots = assets
                .iter()
                .map(|a| RemoteShot {
                    id: a.id.clone(),
                    stem: a.name.clone(),
                    assets: vec![RemoteAsset {
                        object_id: a.id.clone(),
                        name: a.name.clone(),
                        kind: match a.kind {
                            Kind::Jpeg => MtpMediaKind::Jpeg,
                            Kind::Raw => MtpMediaKind::Raw,
                            Kind::Video => MtpMediaKind::Video,
                            Kind::Other => MtpMediaKind::Heif,
                        },
                        size: a.size,
                        preview_path: a.path.clone(),
                        source_path: a.name.clone(),
                        modified: None,
                        cache_key: a.key.clone(),
                    }],
                })
                .collect::<Vec<_>>();
            let plan = plan_import(shots, &destinations).map_err(|error| eyre!(error))?;
            let total = plan.iter().map(|(a, _)| a.size).sum::<u64>();
            let mut completed = 0;
            let mut copied = 0;
            let mut skipped = 0;
            for (asset, destination) in plan {
                let source = asset
                    .preview_path
                    .as_ref()
                    .ok_or_else(|| eyre!("missing source {}", asset.name))?;
                let mut input = std::fs::File::open(source)?;
                if review::local_identity(source, &input.metadata()?) != asset.cache_key {
                    bail!(
                        "{} changed after indexing; reopen the source and review it again",
                        asset.name
                    );
                }
                let source_metadata = input.try_clone()?;
                let mut gate = crate::transfer_progress::ProgressGate::new();
                let outcome = copy_reader_with_progress(
                    &mut input,
                    asset.size,
                    &destination,
                    &asset.name,
                    |done| {
                        if thread_cancel.load(Ordering::Relaxed) {
                            bail!("Import cancelled; completed destination files are preserved");
                        }
                        if done == asset.size
                            && review::local_identity(source, &source_metadata.metadata()?)
                                != asset.cache_key
                        {
                            bail!(
                                "{} changed while copying; reopen and review it again",
                                asset.name
                            );
                        }
                        if gate.ready(done, asset.size) {
                            let _ = sender.send(ImportUpdate::Progress {
                                name: asset.name.clone(),
                                done: completed + done,
                                total,
                            });
                        }
                        Ok(())
                    },
                )?;
                completed += asset.size;
                match outcome {
                    CopyOutcome::Copied => copied += 1,
                    CopyOutcome::SkippedExisting => skipped += 1,
                }
            }
            Ok((copied, skipped))
        })();
        let _ = sender.send(ImportUpdate::Finished(result.map_err(|e| format!("{e:#}"))));
    });
    operation
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::{Decision, History, Settings, scan_local};
    fn finish(operation: ImportOperation) -> std::result::Result<(usize, usize), String> {
        loop {
            match operation
                .receiver
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap()
            {
                ImportUpdate::Finished(result) => return result,
                ImportUpdate::Progress { .. } => {}
            }
        }
    }
    #[test]
    fn only_selected_pairs_and_videos_import_and_changed_sources_are_rejected() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        for name in ["a.JPG", "a.RAF", "b.JPG", "b.RAF", "clip.MOV"] {
            std::fs::write(source.path().join(name), name.as_bytes()).unwrap();
        }
        let mut session = scan_local(source.path().to_owned(), None).unwrap();
        let index = session.shots.iter().position(|s| s.name == "a").unwrap();
        History::default().apply(
            &mut session,
            &[index],
            Some(Kind::Jpeg),
            true,
            Decision::Keep,
        );
        let ids = session.selected_ids(&Settings::default());
        let assets = session
            .shots
            .iter()
            .flat_map(|s| &s.assets)
            .filter(|a| ids.contains(&a.id))
            .cloned()
            .collect::<Vec<_>>();
        let destinations = ImportPaths {
            pictures: Some(target.path().join("jpeg")),
            raw: Some(target.path().join("raw")),
            videos: Some(target.path().join("video")),
            date: chrono::NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            album_name: None,
        };
        assert_eq!(
            finish(start(assets.clone(), destinations.clone())).unwrap(),
            (3, 0)
        );
        assert_eq!(
            finish(start(assets.clone(), destinations.clone())).unwrap(),
            (0, 3)
        );
        std::fs::write(source.path().join("a.JPG"), b"changed file").unwrap();
        let untouched = tempfile::tempdir().unwrap();
        let destinations = ImportPaths {
            pictures: Some(untouched.path().join("jpeg")),
            raw: Some(untouched.path().join("raw")),
            videos: Some(untouched.path().join("video")),
            ..destinations
        };
        assert!(
            finish(start(assets, destinations))
                .unwrap_err()
                .contains("changed after indexing")
        );
        assert_eq!(std::fs::read_dir(untouched.path()).unwrap().count(), 0);
    }
}
