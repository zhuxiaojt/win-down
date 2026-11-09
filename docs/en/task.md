# DownloadTask API

`DownloadTask` represents a single download task. It extends `EventEmitter` and is created by `Downloader.addTask()`.

```ts
const task = downloader.addTask({
  url: 'https://example.com/file.zip',
  destination: 'D:/Downloads/file.zip',
  checksum: { algorithm: 'sha256', value: 'abc...' },
});

task.on('progress', (e) => console.log(e.percent));

await task.wait(); // resolves with ProgressEvent on completion; rejects with DownloadError on failure/cancel
```

## DownloadTaskOptions

| Option | Type | Default | Description |
| --- | --- | --- | --- |
| `url` | `string` | (required) | Download URL |
| `destination` | `string` | (required) | Destination file path |
| `mirrors` | `string[]` | `[]` | Mirror URLs, tried in order when the primary URL fails |
| `headers` | `Record<string, string>` | - | Per-task request headers (CR/LF forbidden) |
| `expectedSize` | `number` | `-1` | Expected file size, verified after download (-1 skips the check) |
| `checksum` | `{ algorithm, value }` | - | Integrity check; `algorithm` is `'sha256' \| 'md5' \| 'crc32'` and `value` is the hex digest |
| `priority` | `number` | `0` | Queue priority; higher values are scheduled first |
| `maxConcurrency` | `number` | global value | Per-task concurrency override |
| `overwrite` | `'error' \| 'overwrite' \| 'rename' \| 'skip'` | `'error'` | Behavior when the destination file already exists |
| `resume` | `boolean` | global value | Per-task resume override |
| `segmentSize` | `number` | global value | Per-task segment size override |

### overwrite modes

| Mode | Behavior |
| --- | --- |
| `error` | Task fails when the file exists (`ERR_FILE_EXISTS`) |
| `overwrite` | Overwrite the existing file |
| `rename` | Automatically rename (e.g. `file (1).zip`) |
| `skip` | Skip the download; the task completes immediately (`skipped` is `true`) |

### Example: fully configured task

```ts
const task = downloader.addTask({
  url: 'https://mirror1.example.com/app-1.0.zip',
  // mirrors are tried automatically if the primary source fails
  mirrors: [
    'https://mirror2.example.com/app-1.0.zip',
    'https://mirror3.example.com/app-1.0.zip',
  ],
  destination: 'D:/Downloads/app-1.0.zip',
  headers: { Authorization: 'Bearer <token>' },
  expectedSize: 104857600, // verify the actual size after download
  checksum: { algorithm: 'sha256', value: 'e3b0c44298fc1c14...' },
  priority: 10,            // high priority, scheduled before regular tasks
  maxConcurrency: 16,      // extra concurrency for this task alone
  overwrite: 'rename',     // rename if the file already exists
  resume: true,            // enable resumable downloads
});
```

### Example: skip vs rename

```ts
// If D:/Downloads/report.pdf already exists:
// error (default) → task fails with code ERR_FILE_EXISTS
// rename          → downloads to report (1).pdf
// skip            → no download; the task completes and task.skipped === true

const t = downloader.addTask({
  url: 'https://example.com/report.pdf',
  destination: 'D:/Downloads/report.pdf',
  overwrite: 'skip',
});
await t.wait();
console.log(t.skipped); // true (if the file was already there)
```

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `id` | `string` | Task ID |
| `url` / `destination` | `string` | URL and destination passed at creation |
| `state` | `TaskState` | Current state, see below |
| `done` | `boolean` (getter) | Whether the task is in a terminal state |
| `totalBytes` | `number \| null` | Total file size (`null` when unknown) |
| `downloadedBytes` | `number` | Bytes downloaded (including previous sessions) |
| `percent` | `number \| null` | Progress percentage (`null` when unknown) |
| `speedBps` | `number` | Instantaneous speed (B/s) |
| `etaSeconds` | `number \| null` | Estimated time remaining (seconds) |
| `activeConnections` | `number` | Currently active connections |
| `segmentsTotal` / `segmentsCompleted` | `number` | Total / completed segment counts |
| `degraded` | `boolean` | Whether the task is in degraded mode |
| `degradeReason` | `DegradeReason \| null` | Degradation reason, see [errors.md](errors.md) |
| `checksumVerified` | `boolean \| null` | Checksum result (`null` when no checksum configured) |
| `skipped` | `boolean` | Skipped due to `overwrite: 'skip'` |
| `attempts` | `number` | Retry attempts so far |
| `elapsedMs` | `number` | Task elapsed time (ms) |
| `resumedBytes` | `number` | Bytes restored from a previous session |
| `sessionBytes` | `number` | Bytes downloaded in the current session |
| `error` | `DownloadError \| null` | Error object when the task failed |
| `finalUrl` / `etag` / `lastModified` / `contentType` | `string \| null` | Metadata from probing/response |
| `downloader` | `Downloader` | Owning downloader instance |

### Example: rendering a live progress bar

```ts
task.on('progress', (e) => {
  const bar = '█'.repeat(Math.floor(e.percent / 5)) + '░'.repeat(20 - Math.floor(e.percent / 5));
  process.stdout.write(
    `\r${bar} ${e.percent.toFixed(1)}%  ` +
    `${(e.downloadedBytes / 1e6).toFixed(0)}/${((e.totalBytes ?? 0) / 1e6).toFixed(0)}MB  ` +
    `${(e.speedBps / 1e6).toFixed(1)}MB/s  ETA ${e.etaSeconds}s  ` +
    `conn ${e.activeConnections} segs ${e.segmentsCompleted}/${e.segmentsTotal} `
  );
});
```

### Example: reading final task info

```ts
await task.wait();
console.log({
  state: task.state,              // 'completed'
  checksumVerified: task.checksumVerified, // true
  elapsedMs: task.elapsedMs,      // total elapsed time
  resumedBytes: task.resumedBytes, // bytes restored from resume (0 = fresh download)
  finalUrl: task.finalUrl,        // final URL after redirects
});
```

## TaskState

State machine: `queued → probing → downloading ⇄ retrying → verifying → completed`; `paused` / `failed` / `cancelled` can be entered from any stage.

| State | Description |
| --- | --- |
| `queued` | Waiting in queue |
| `probing` | Probing URL metadata |
| `downloading` | Downloading |
| `retrying` | Waiting to retry |
| `paused` | Paused |
| `verifying` | Verifying size/checksum |
| `completed` | Completed (terminal) |
| `failed` | Failed (terminal) |
| `cancelled` | Cancelled (terminal) |

### Example: listening for state changes

```ts
task.on('state', (e) => {
  if (e.state === 'paused') console.log('Paused; progress saved');
  if (e.state === 'verifying') console.log('Download finished; verifying...');
});
```

## Methods

| Method | Description |
| --- | --- |
| `wait(): Promise<ProgressEvent>` | Wait for the task to finish; resolves with the final progress event on completion, rejects with `DownloadError` on failure/cancel. Can be called multiple times |
| `pause()` / `resume()` | Pause / resume |
| `cancel({ removePartialFile? }?)` | Cancel the task, optionally deleting the unfinished fragment |
| `on(event, listener)` | Listen for events (fully typed overloads) |

### Example: the three outcomes of wait

```ts
// 1. Successful completion
const result = await task.wait();

// 2. Failure
try { await failingTask.wait(); }
catch (err) { console.log(err.code, err.message); }

// 3. Cancellation
try { await cancelledTask.wait(); }
catch (err) { console.log(err.code); } // 'ERR_CANCELLED'
```

### Example: auto-cancel after a timeout

```ts
const task = downloader.addTask({ url, destination });

const timer = setTimeout(() => task.cancel({ removePartialFile: true }), 10 * 60_000);
try {
  await task.wait();
} finally {
  clearTimeout(timer);
}
```

## Events

| Event | Arguments | Description |
| --- | --- | --- |
| `progress` | `ProgressEvent` | Progress updated |
| `state` | `ProgressEvent` | State changed |
| `degraded` | `DegradedEvent` | Entered degraded mode |
| `reset` | `ProgressEvent` | Progress reset (e.g. re-downloading from scratch after a validator change) |
| `completed` | `ProgressEvent` | Completed |
| `failed` | `(DownloadError, ProgressEvent)` | Failed |
| `cancelled` | - | Cancelled |

## ProgressEvent

Progress and result events all carry the following fields:

| Field | Description |
| --- | --- |
| `kind` | `'progress' \| 'state' \| 'reset' \| 'completed' \| 'failed'` |
| `taskId` / `url` / `destination` | Task identity |
| `state` | Current `TaskState` |
| `totalBytes` / `downloadedBytes` | Total size / bytes downloaded |
| `sessionBytes` / `resumedBytes` | Current-session bytes / restored bytes |
| `percent` | Percentage (-1 when unknown) |
| `speedBps` / `averageSpeedBps` | Instantaneous / average speed |
| `etaSeconds` | Estimated seconds remaining |
| `activeConnections` | Active connection count |
| `segmentsTotal` / `segmentsCompleted` | Segment progress |
| `degraded` / `degradeReason` | Degradation flag and reason |
| `elapsedMs` / `attempts` / `connectionsUsed` | Elapsed time / retry attempts / total connections used |
| `checksumVerified` / `skipped` | Checksum result / whether skipped |
| `finalUrl` / `etag` / `lastModified` / `contentType` | Response metadata |
| `code` / `message` | Error code and message when failed |

## Resumable downloads

When `resume` is enabled (globally or per task):

- If a task is paused or the process exits, re-adding a task with the same URL and destination resumes based on downloaded bytes and segment progress
- If the server's ETag / Last-Modified changes, the download automatically restarts from scratch (triggering a `reset` event with degradation reason `validator-changed`)
- The `resumedBytes` field shows how many bytes were actually restored

### Example: resuming after a process restart

```ts
// First run: download interrupted halfway by Ctrl+C
// Next run: the same url + destination automatically resumes from the breakpoint
const task = downloader.addTask({
  url: 'https://example.com/big.iso',
  destination: 'D:/Downloads/big.iso',
  resume: true,
});
task.on('progress', (e) => {
  if (e.resumedBytes > 0) console.log(`Resumed ${(e.resumedBytes / 1e6).toFixed(0)}MB from breakpoint`);
});
await task.wait();
```

## Related docs

- [Downloader API](downloader.md)
- [Error codes and degradation reasons](errors.md)
