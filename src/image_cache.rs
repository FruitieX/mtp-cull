use crate::focus::FocusMap;
use color_eyre::eyre::{Result, bail};
use eframe::egui;
use image::ImageDecoder;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::{Duration, Instant};

pub const QUICK_PREVIEW_EDGE: u32 = 128;

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub struct Key {
    pub path: PathBuf,
    pub edge: Option<u32>,
    pub analyze: bool,
}
impl Key {
    /// Reuse only this file's pixels. Reel uploads exclude large/native buffers.
    pub fn preview_rank(&self, requested: &Self, thumbnail: bool) -> Option<(bool, u32)> {
        if self.path != requested.path || (thumbnail && self.edge.is_none_or(|edge| edge > 1024)) {
            return None;
        }
        Some((self == requested, self.edge.unwrap_or(u32::MAX)))
    }
    fn tier(&self) -> usize {
        match self.edge {
            Some(edge) if edge <= 256 => 0,
            Some(_) => 1,
            None => 2,
        }
    }
    pub fn native(path: PathBuf) -> Self {
        Self {
            path,
            edge: None,
            analyze: false,
        }
    }
    pub fn focus(path: PathBuf) -> Self {
        Self {
            path,
            edge: None,
            analyze: true,
        }
    }
    pub fn fit(path: PathBuf, edge: u32) -> Self {
        Self {
            path,
            edge: Some(edge),
            analyze: false,
        }
    }
}
pub struct Picture {
    pub image: Arc<egui::ColorImage>,
    pub source_size: [usize; 2],
    pub focus: Option<Arc<FocusMap>>,
    pub decode_time: Duration,
    pub feature: crate::bursts::Feature,
}
impl Picture {
    fn bytes(&self) -> usize {
        self.image.pixels.len() * 4 + self.focus.as_ref().map_or(0, |f| f.energy.len())
    }
}
struct Entry {
    picture: Arc<Picture>,
    tick: u64,
}
#[derive(Clone)]
struct Job {
    key: Key,
    priority: u8,
    generation: u64,
}
#[derive(Default)]
struct Queue {
    jobs: Vec<Job>,
    in_flight: HashSet<Key>,
    generation: u64,
    stop: bool,
}
type Shared = Arc<(Mutex<Queue>, Condvar)>;
struct Completed {
    key: Key,
    generation: u64,
    result: std::result::Result<Picture, String>,
}
#[derive(Default)]
pub struct Statistics {
    pub hits: u64,
    pub misses: u64,
    pub decodes: u64,
    pub decode_ms: f64,
}
pub struct ImageCache {
    queue: Shared,
    receiver: mpsc::Receiver<Completed>,
    entries: HashMap<Key, Entry>,
    failures: HashMap<Key, String>,
    workers: Vec<std::thread::JoinHandle<()>>,
    generation: u64,
    tick: u64,
    budget: usize,
    pub stats: Statistics,
}
impl ImageCache {
    pub fn new(budget: usize) -> Self {
        let queue: Shared = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        // Bounded completed images prevent decoding from outrunning UI consumption.
        let (sender, receiver) = mpsc::sync_channel::<Completed>(4);
        let count = std::thread::available_parallelism()
            .map_or(4, usize::from)
            .clamp(2, 6);
        let workers = (0..count)
            .map(|i| {
                let shared = queue.clone();
                let sender = sender.clone();
                std::thread::Builder::new()
                    .name(format!("jpeg-decode-{i}"))
                    .spawn(move || {
                        loop {
                            let job = {
                                let (mutex, wake) = &*shared;
                                let mut queue = mutex.lock().unwrap();
                                loop {
                                    if queue.stop {
                                        return;
                                    }
                                    if let Some(job) = queue.jobs.pop() {
                                        if job.generation == queue.generation
                                            && queue.in_flight.insert(job.key.clone())
                                        {
                                            break job;
                                        }
                                    } else {
                                        queue = wake.wait(queue).unwrap();
                                    }
                                }
                            };
                            let result = decode(&job.key).map_err(|e| format!("{e:#}"));
                            if sender
                                .send(Completed {
                                    key: job.key.clone(),
                                    generation: job.generation,
                                    result,
                                })
                                .is_err()
                            {
                                return;
                            }
                        }
                    })
                    .expect("could not start JPEG worker")
            })
            .collect();
        Self {
            queue,
            receiver,
            entries: HashMap::new(),
            failures: HashMap::new(),
            workers,
            generation: 0,
            tick: 0,
            budget,
            stats: Statistics::default(),
        }
    }
    pub fn clear(&mut self) {
        self.generation += 1;
        self.entries.clear();
        self.failures.clear();
        let mut queue = self.queue.0.lock().unwrap();
        queue.generation = self.generation;
        queue.jobs.clear();
        queue.in_flight.clear();
    }
    pub fn set_budget(&mut self, bytes: usize) {
        self.budget = bytes.max(64 * 1024 * 1024);
    }
    pub fn bytes(&self) -> usize {
        self.entries.values().map(|e| e.picture.bytes()).sum()
    }
    pub fn pending(&self) -> bool {
        let queue = self.queue.0.lock().unwrap();
        !queue.jobs.is_empty() || !queue.in_flight.is_empty()
    }
    pub fn get(&mut self, key: &Key) -> Option<Arc<Picture>> {
        if let Some(entry) = self.entries.get_mut(key) {
            self.tick += 1;
            entry.tick = self.tick;
            self.stats.hits += 1;
            Some(entry.picture.clone())
        } else {
            self.stats.misses += 1;
            None
        }
    }
    pub fn failure(&self, key: &Key) -> Option<&str> {
        self.failures.get(key).map(String::as_str)
    }
    pub fn has_failures(&self) -> bool {
        !self.failures.is_empty()
    }
    pub fn preview(&mut self, key: &Key, thumbnail: bool) -> Option<(Key, Arc<Picture>)> {
        if self.entries.contains_key(key) {
            return self.get(key).map(|picture| (key.clone(), picture));
        }
        let best = self
            .entries
            .keys()
            .filter_map(|candidate| {
                candidate
                    .preview_rank(key, thumbnail)
                    .map(|rank| (candidate, rank))
            })
            .max_by_key(|(_, rank)| *rank)
            .map(|(candidate, _)| candidate.clone())?;
        self.get(&best).map(|picture| (best, picture))
    }
    pub fn feature(&self, key: &Key) -> Option<crate::bursts::Feature> {
        self.entries.get(key).map(|e| e.picture.feature)
    }
    pub fn source_size(&self, path: &std::path::Path) -> Option<[usize; 2]> {
        self.entries
            .iter()
            .find(|(k, _)| k.path == path)
            .map(|(_, e)| e.picture.source_size)
    }
    pub fn retry(&mut self) {
        self.failures.clear();
    }
    pub fn poll(&mut self, pins: &HashSet<Key>) {
        while let Ok(done) = self.receiver.try_recv() {
            if done.generation != self.generation {
                continue;
            }
            self.queue.0.lock().unwrap().in_flight.remove(&done.key);
            match done.result {
                Ok(picture) => {
                    self.stats.decodes += 1;
                    self.stats.decode_ms += picture.decode_time.as_secs_f64() * 1000.0;
                    self.tick += 1;
                    self.entries.insert(
                        done.key,
                        Entry {
                            picture: Arc::new(picture),
                            tick: self.tick,
                        },
                    );
                }
                Err(message) => {
                    self.failures.insert(done.key, message);
                }
            }
        }
        let limits = [self.budget / 20, self.budget / 5, self.budget * 3 / 4];
        for (tier, limit) in limits.into_iter().enumerate() {
            while self
                .entries
                .iter()
                .filter(|(k, _)| k.tier() == tier)
                .map(|(_, e)| e.picture.bytes())
                .sum::<usize>()
                > limit
            {
                let oldest = self
                    .entries
                    .iter()
                    .filter(|(key, _)| key.tier() == tier && !pins.contains(*key))
                    .min_by_key(|(_, entry)| entry.tick)
                    .map(|(key, _)| key.clone());
                if let Some(key) = oldest {
                    self.entries.remove(&key);
                } else {
                    break;
                }
            }
        }
    }
    /// Replace demand, so a jump immediately supersedes queued neighbors.
    pub fn demand(&self, mut requests: Vec<(Key, u8)>) {
        // Select by urgency before bounding/deduplicating the queue. Background
        // burst analysis must not crowd out visible quick previews.
        requests
            .sort_by_key(|(key, priority)| (*priority, key.edge.unwrap_or(u32::MAX), key.analyze));
        let mut queue = self.queue.0.lock().unwrap();
        let mut seen = HashSet::new();
        queue.jobs = requests
            .into_iter()
            .filter(|(key, _)| {
                seen.insert(key.clone())
                    && !self.entries.contains_key(key)
                    && !self.failures.contains_key(key)
                    && !queue.in_flight.contains(key)
            })
            .take(128)
            .map(|(key, priority)| Job {
                key,
                priority,
                generation: self.generation,
            })
            .collect();
        queue.jobs.sort_by_key(|job| {
            std::cmp::Reverse((
                job.priority,
                job.key.edge.unwrap_or(u32::MAX),
                job.key.analyze,
            ))
        });
        self.queue.1.notify_all();
    }
}
impl Drop for ImageCache {
    fn drop(&mut self) {
        self.queue.0.lock().unwrap().stop = true;
        self.queue.1.notify_all();
        // Keep draining the bounded completion channel until decoding workers exit.
        while self.workers.iter().any(|w| !w.is_finished()) {
            let _ = self.receiver.recv_timeout(Duration::from_millis(10));
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

pub fn decode(key: &Key) -> Result<Picture> {
    let start = Instant::now();
    let mut reader = image::ImageReader::open(&key.path)?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(1024 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    if width as u64 * height as u64 > 100_000_000 {
        bail!("image exceeds 100 megapixel preview limit");
    }
    let orientation = decoder.orientation()?;
    let time = crate::bursts::capture_time(decoder.exif_metadata().ok().flatten());
    #[cfg(not(feature = "turbo"))]
    let mut image = image::DynamicImage::from_decoder(decoder)?;
    #[cfg(feature = "turbo")]
    let mut image = {
        drop(decoder);
        let bytes = std::fs::read(&key.path)?;
        let mut decoder = turbojpeg::Decompressor::new()?;
        let header = decoder.read_header(&bytes)?;
        let factor = key
            .edge
            .and_then(|edge| {
                turbojpeg::Decompressor::supported_scaling_factors()
                    .into_iter()
                    .filter(|f| f.scale(header.width.max(header.height)) >= edge as usize)
                    .min_by_key(|f| f.scale(header.width.max(header.height)))
            })
            .unwrap_or(turbojpeg::ScalingFactor::ONE);
        decoder.set_scaling_factor(factor)?;
        let scaled = header.scaled(factor);
        let mut output = turbojpeg::Image {
            pixels: vec![0; scaled.width * scaled.height * 4],
            width: scaled.width,
            height: scaled.height,
            pitch: scaled.width * 4,
            format: turbojpeg::PixelFormat::RGBA,
        };
        decoder.decompress(&bytes, output.as_deref_mut())?;
        image::DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(scaled.width as u32, scaled.height as u32, output.pixels)
                .ok_or_else(|| color_eyre::eyre::eyre!("invalid JPEG dimensions"))?,
        )
    };
    image.apply_orientation(orientation);
    use image::metadata::Orientation;
    let source_size = if matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    ) {
        [height as usize, width as usize]
    } else {
        [width as usize, height as usize]
    };
    let focus = key
        .analyze
        .then(|| Arc::new(FocusMap::compute(&image.to_luma8())));
    let image = if let Some(edge) = key.edge {
        image.thumbnail(edge, edge)
    } else {
        image
    };
    let feature = crate::bursts::Feature {
        time,
        hash: if key.edge.is_some() {
            crate::bursts::similarity(&image)
        } else {
            0
        },
    };
    let rgba = image.into_rgba8();
    let image = Arc::new(egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ));
    Ok(Picture {
        image,
        source_size,
        focus,
        decode_time: start.elapsed(),
        feature,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn small_picture() -> Arc<Picture> {
        Arc::new(Picture {
            image: Arc::new(egui::ColorImage::filled([10, 10], egui::Color32::BLACK)),
            source_size: [10, 10],
            focus: None,
            decode_time: Duration::ZERO,
            feature: crate::bursts::Feature::default(),
        })
    }
    #[test]
    fn previews_reuse_this_photo_across_sizes_and_keep_large_buffers_out_of_the_reel() {
        let mut cache = ImageCache::new(64 * 1024 * 1024);
        let requested = Key::fit("photo.jpg".into(), 384);
        for key in [
            Key::fit("other.jpg".into(), 384),
            Key::fit("photo.jpg".into(), 128),
            Key::fit("photo.jpg".into(), 256),
            Key::native("photo.jpg".into()),
        ] {
            cache.entries.insert(
                key,
                Entry {
                    picture: small_picture(),
                    tick: 0,
                },
            );
        }
        assert_eq!(cache.preview(&requested, true).unwrap().0.edge, Some(256));
        assert_eq!(cache.preview(&requested, false).unwrap().0.edge, None);
        cache.entries.insert(
            requested.clone(),
            Entry {
                picture: small_picture(),
                tick: 0,
            },
        );
        assert_eq!(cache.preview(&requested, true).unwrap().0, requested);
        assert!(
            cache
                .preview(&Key::fit("missing.jpg".into(), 128), false)
                .is_none()
        );
    }
    #[test]
    fn cold_demands_prioritize_active_quick_previews_before_native_or_neighbor_decodes() {
        let cache = ImageCache::new(64 * 1024 * 1024);
        cache.queue.0.lock().unwrap().stop = true;
        let quick = Key::fit("active.jpg".into(), QUICK_PREVIEW_EDGE);
        cache.demand(vec![
            (Key::native("active.jpg".into()), 0),
            (Key::fit("active.jpg".into(), 2048), 0),
            (quick.clone(), 0),
            (Key::fit("neighbor.jpg".into(), 128), 1),
        ]);
        let mut queue = cache.queue.0.lock().unwrap();
        assert_eq!(queue.jobs.pop().unwrap().key, quick);
        assert_eq!(queue.jobs.pop().unwrap().key.edge, Some(2048));
        assert_eq!(queue.jobs.pop().unwrap().key.edge, None);
        assert_eq!(queue.jobs.pop().unwrap().priority, 1);
    }
    #[test]
    fn background_requests_cannot_crowd_visible_previews_out_of_the_bounded_queue() {
        let cache = ImageCache::new(64 * 1024 * 1024);
        cache.queue.0.lock().unwrap().stop = true;
        let visible = Key::fit("visible.jpg".into(), 128);
        let mut requests = (0..500)
            .map(|i| (Key::fit(format!("background-{i}.jpg").into(), 512), 4))
            .collect::<Vec<_>>();
        requests.push((visible.clone(), 4));
        requests.push((visible.clone(), 3));
        cache.demand(requests);
        let mut queue = cache.queue.0.lock().unwrap();
        assert_eq!(queue.jobs.len(), 128);
        let first = queue.jobs.pop().unwrap();
        assert_eq!(first.key, visible);
        assert_eq!(first.priority, 3);
    }
    #[test]
    fn tiers_evict_independently_and_active_buffers_survive() {
        let mut cache = ImageCache::new(10000);
        let mut pins = HashSet::new();
        for tier in 0..3 {
            for i in 0..4 {
                let key = if tier == 2 {
                    Key::native(format!("{tier}/{i}").into())
                } else {
                    Key::fit(
                        format!("{tier}/{i}").into(),
                        if tier == 0 { 192 } else { 1024 },
                    )
                };
                if tier == 0 && i == 0 {
                    pins.insert(key.clone());
                }
                cache.entries.insert(
                    key,
                    Entry {
                        picture: small_picture(),
                        tick: i,
                    },
                );
            }
        }
        cache.poll(&pins);
        assert_eq!(cache.entries.keys().filter(|k| k.tier() == 0).count(), 1);
        assert_eq!(cache.entries.keys().filter(|k| k.tier() == 1).count(), 4);
        assert_eq!(cache.entries.keys().filter(|k| k.tier() == 2).count(), 4);
        assert!(pins.iter().all(|p| cache.entries.contains_key(p)));
    }
    #[test]
    fn stale_decode_cannot_replace_new_image_or_remove_new_inflight_job() {
        let mut cache = ImageCache::new(10000);
        cache.clear();
        let key = Key::native("same.jpg".into());
        cache.queue.0.lock().unwrap().in_flight.insert(key.clone());
        let (sender, receiver) = mpsc::channel();
        cache.receiver = receiver;
        sender
            .send(Completed {
                key: key.clone(),
                generation: 0,
                result: Err("old failure".into()),
            })
            .unwrap();
        cache.poll(&HashSet::new());
        assert!(cache.failures.is_empty());
        assert!(cache.queue.0.lock().unwrap().in_flight.contains(&key));
    }
    #[test]
    fn exif_rotation_is_applied_before_fit_and_preserves_native_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("portrait.jpg");
        image::RgbImage::from_pixel(40, 60, image::Rgb([20, 40, 80]))
            .save(&path)
            .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        let mut oriented = bytes[..2].to_vec();
        oriented.extend_from_slice(&[255, 225]);
        oriented.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        oriented.extend_from_slice(exif);
        oriented.extend_from_slice(&bytes[2..]);
        std::fs::write(&path, oriented).unwrap();
        let picture = decode(&Key::fit(path.clone(), 20)).unwrap();
        assert_eq!(picture.source_size, [60, 40]);
        assert_eq!(picture.image.size, [20, 13]);
        let native = decode(&Key::native(path)).unwrap();
        assert_eq!(native.image.size, [60, 40]);
    }
    #[test]
    fn decoding_preserves_native_dimensions_for_scaled_preview() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.jpg");
        image::RgbImage::from_pixel(320, 200, image::Rgb([20, 30, 40]))
            .save(&path)
            .unwrap();
        let picture = decode(&Key::fit(path, 80)).unwrap();
        assert_eq!(picture.source_size, [320, 200]);
        assert_eq!(picture.image.size, [80, 50]);
    }
    #[test]
    fn queued_demand_is_bounded_and_replaced_after_navigation() {
        let cache = ImageCache::new(64 * 1024 * 1024);
        let mut queue = cache.queue.0.lock().unwrap();
        queue.stop = true;
        drop(queue);
        cache.demand(
            (0..300)
                .map(|i| (Key::fit(PathBuf::from(format!("{i}.jpg")), 100), 3))
                .collect(),
        );
        assert_eq!(cache.queue.0.lock().unwrap().jobs.len(), 128);
        cache.demand(vec![(Key::fit("new.jpg".into(), 100), 0)]);
        queue = cache.queue.0.lock().unwrap();
        assert_eq!(queue.jobs.len(), 1);
        assert_eq!(queue.jobs[0].key.path, PathBuf::from("new.jpg"));
    }
}
