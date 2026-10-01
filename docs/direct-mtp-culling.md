# Direct camera review and import

`app.rs` communicates with a dedicated worker through `MtpRequest`/`MtpEvent`.
Windows Portable Devices and Linux `mtp-rs 0.32.0` handles stay on that worker
thread. The UI never opens camera streams or invokes camera writes/deletes.

## Index and staging

Starting a session recursively enumerates media metadata. Assets group by relative
folder and exact filename stem, so duplicate stems in separate folders never pair.
Linux rejects incomplete listings instead of silently omitting unreadable objects.

Once indexed, the worker stages whole-session JPEG originals. Active A/B and nearby
shots have priority. UI requests replace the pending priority list; at most one
camera transfer runs at a time. Pause takes effect at the next transfer boundary;
cancel checks run between Windows stream chunks or Linux 8 MiB byte ranges.
Progress notifications are throttled to 40 ms while cancellation remains frequent.

Session generations accompany every request/event. Changing/closing the source
invalidates old results. Native OS/driver calls may block and cannot be forcibly
interrupted. Closing the UI signals cancellation without waiting indefinitely for
a blocked native call; the worker releases handles when that call returns.

## Persistence and quota

Staging lives in the per-user application data directory under `mtp-staging/`.
Each camera/folder directory and JPEG filename is hashed. A session manifest and
verification sidecar accompany cached originals. Review decisions live separately
in `cull.sqlite3`; import success does not immediately discard the session cache.

Keys combine backend device locator, selected source folder, relative file path,
size and camera modification/creation timestamp. Unknown timestamps use a fresh
session nonce: the UI warns that these decisions cannot be trusted on reconnect.
Changed keys are left unreviewed and unmatched previous records are reported.
Device locators are not portable serial numbers; moving a Linux USB device to a
different port may create a new review identity. Identical metadata is not a
cryptographic proof of unchanged source content.

Cached browse reuse checks key, size and local modification time. Staging computes
BLAKE3 while streaming; import checks that hash before publishing a cached JPEG.
An altered cache therefore cannot silently publish a different JPEG under a kept
decision. Metadata verification avoids rereading all staged originals at startup.

The disk quota applies across recognized staging directories. Oldest inactive
sessions are evicted when space is needed; inactive sessions older than 30 days
are maintained on the next transfer. Active-session JPEGs stay pinned. Reindexing
removes orphaned hashed files from that session. Cleanup only deletes flat,
internally named files/directories, rejects paths outside the cache, and never
recursively traverses user folders or source media. Review database entries survive
staging eviction; JPEGs can be fetched again on reconnect.

## Import

The UI sends explicit selected asset IDs. The worker validates those IDs and plans
all destinations before copying. JPEG decisions select corresponding RAW assets
when linking is enabled; independent RAW choices are retained. Videos default to
Keep but the import include-videos switch can exclude them. Filters do not affect
the selected asset list. Conflicting pairs are excluded and visibly reported.

Selected staged JPEGs copy from disk with hash verification; selected RAW/video
companions transfer from the camera. Unselected companions never transfer. Missing
destination roots, path traversal and case-insensitive destination collisions fail
preflight before any destination file is written.

Every file uses `safe_copy.rs`: same-directory temporary file, expected byte count,
flush/sync and atomic no-clobber publication. Identical existing files are compared
byte-for-byte and skipped; differing files are never overwritten. Cancellation or
failure removes the current temporary file and preserves completed copies/cache.
Retry may reread a companion to prove that an existing destination is identical.
Local imports additionally recheck source size/mtime identity before copying and
before publication.

## Verification boundary

Fake-worker tests exercise priority, quota pause/resume and stale event rejection.
Transfer tests cover cancellation, incomplete data, hash mismatch, retry and
destination collisions. Windows builds and native GPU UI smoke tests pass. The
Linux adapter also compiles against `mtp-rs 0.32.0` in a Windows-hosted API harness;
this is not a native Linux/USB integration test.

Physical X-T5, NAS and native Linux checks remain: enumerate a 200-500-shot album,
disconnect during staging/import, reconnect and reconcile choices, exercise quota,
and verify that exactly the selected JPEG/RAF/video originals arrive at the NAS.
