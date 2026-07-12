# Direct MTP Culling

The Windows and Linux implementations preserve the local workflow's safety properties while keeping all device I/O off the UI thread. macOS remains unsupported.

## Workflow

1. The UI lists connected devices and lets the user choose one source folder.
2. A single worker, which exclusively owns the opened MTP device, enumerates recognized media and groups exact filename stems.
3. The worker downloads JPEG companions to a session-specific temporary application-data cache. RAW, HEIF, video, rejected, and unrated files stay on the device.
4. The local culling UI uses cached JPEGs. Decisions are session-local until import because device objects do not have portable content fingerprints without downloading their full contents.
5. An import review asks for the picture, RAW, and video roots plus album name/date. Only explicit Keep pairs are copied.
6. Each imported file uses the existing temporary-file, byte-count, duplicate-content, and no-clobber rules. The worker reports per-file outcomes.
7. Cache cleanup happens only after every selected Keep file imports successfully. Failed imports retain the cache for retry. MTP files are never deleted.

## Backend Boundary

The worker-owned session boundary keeps `winmtp::Object` out of the UI:

```text
MtpBackend -> open_session(device) -> MtpSession worker
UI <-> request/response channel <-> MtpSession worker
```

The implemented request set is:

- `ListDevices`
- `ListSourceFolders`
- `StartSession { device, source_folder }`
- `ImportKept { items, destinations }`
- `Shutdown`

`StartSession` caches all JPEG companions for that source folder. Each listed item contains backend-neutral IDs, filename, media kind, size, and exact-stem group ID. The Windows worker keeps the matching COM objects private. Linux and macOS backends can implement the same requests without leaking platform types into the culling UI.

## Persistence

MTP decisions are intentionally in memory and keyed by short-lived remote-shot IDs. Device objects do not have portable content fingerprints without downloading their full contents. Imported local JPEGs receive the existing BLAKE3 content-fingerprint decisions after their verified copy completes. Persistent resumable MTP sessions remain future work.

## Verification

- Keep-only import planning and collision handling have unit coverage.
- Exercise Windows Portable Devices manually with Fujifilm RAF+JPEG and Pixel DNG+JPEG folders.
- Test disconnects during enumeration, preview caching, and import.
- Confirm that rejected and unrated source objects remain untouched.
- Measure JPEG cache latency and import throughput before enabling direct mode by default.
