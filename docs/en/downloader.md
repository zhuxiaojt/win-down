# Downloader API

`Downloader` is the main entry point. It manages the task queue, concurrency scheduling, and global statistics. It extends `EventEmitter`.

```ts
import { Downloader } from 'win-down';

const downloader = new Downloader({
  maxConcurrency: 8,
  resume: true,
});
```

## Constructor

```ts
new Downloader(options?: DownloaderOptions)
```

### DownloaderOptions

| Option | Type | Default | Description |
| --- | --- | --- | --- |
| `maxConcurrency` | `number` | `16` | Global maximum concurrent connections (1–256) |
| `maxTasksInFlight` | `number` | `4` | Maximum simultaneously downloading tasks (1–64) |
| `policy` | `'balanced' \| 'favor-other-apps' \| 'max-performance'` | `'balanced'` | Resource policy, see [Resource policies](#resource-policies) |
| `segmentSize` | `number` | `8 MiB` | Segment size (bytes) |
| `maxSegmentsPerTask` | `number` | `32` | Maximum segments per task (1–1024) |
| `smallFileThreshold` | `number` | `2 MiB` | Files smaller than this are not segmented and use a single connection |
| `maxPerHost` | `number` | `6` | Maximum connections per host |
| `connectTimeoutMs` | `number` | `10000` | Connect timeout (ms) |
| `idleTimeoutMs` | `number` | `30000` | Idle timeout (ms) |
| `maxRetries` | `number` | `5` | Maximum retry attempts |
| `baseRetryMs` | `number` | `250` | Base backoff interval (ms) |
| `maxRetryMs` | `number` | `30000` | Backoff upper bound (ms) |
| `followRedirects` | `boolean` | `true` | Follow redirects |
| `maxRedirects` | `number` | `10` | Maximum redirects |
| `verifyTls` | `boolean` | `true` | Verify TLS certificates |
| `proxy` | `string` | `'system'` | Proxy URL; defaults to the system proxy |
| `userAgent` | `string` | `'win-down/1.0'` | User-Agent |
| `headers` | `Record<string, string>` | - | Global default request headers (CR/LF forbidden) |
| `resume` | `boolean` | `false` | Global resume switch |
| `preallocate` | `boolean` | `true` | Preallocate file space |
| `progressIntervalMs` | `number` | `100` | Progress event interval (minimum 20ms) |
| `ioBufferSize` | `number` | `512 KiB` | I/O buffer size (64 KiB – 4 MiB) |
| `createDirectories` | `boolean` | `true` | Automatically create destination directories |

## Task management

| Method | Returns | Description |
| --- | --- | --- |
| `addTask(options: DownloadTaskOptions)` | `DownloadTask` | Add a download task. Throws `ERR_CLOSED` after shutdown |
| `getTask(id: string)` | `DownloadTask \| null` | Look up a task by ID |
| `get tasks` | `DownloadTask[]` | Snapshot of all current tasks |

### Example: adding and querying tasks

```ts
const task = downloader.addTask({
  url: 'https://example.com/installer.exe',
  destination: 'D:/Downloads/installer.exe',
});

console.log(task.id);                 // task ID, usable with getTask
console.log(downloader.tasks.length); // total number of current tasks
```

## Task control

| Method | Description |
| --- | --- |
| `pause(id)` | Pause a task |
| `resume(id)` | Resume a task |
| `cancel(id, { removePartialFile? })` | Cancel a task; when `removePartialFile` is `true`, the unfinished file fragment is deleted too |
| `pauseAll()` | Pause all tasks |
| `resumeAll()` | Resume all tasks |
| `cancelAll({ removePartialFile? })` | Cancel all tasks |
| `removeTask(id, { removePartialFile? })` | Cancel and remove a task from the list |
| `clearCompleted()` | Clear all completed/failed/cancelled task records |

### Example: batch tasks with pause/resume

```ts
const urls = [
  'https://example.com/a.zip',
  'https://example.com/b.zip',
  'https://example.com/c.zip',
];

for (const url of urls) {
  downloader.addTask({ url, destination: `D:/Downloads/${url.split('/').pop()}` });
}

// Once all tasks are queued, pause everything
await downloader.pauseAll();

// ...and resume later
await downloader.resumeAll();

// Clean up records of finished tasks
await downloader.clearCompleted();
```

## Global control

| Method | Description |
| --- | --- |
| `getStats(): Stats` | Get a global statistics snapshot |
| `setMaxConcurrency(n)` | Change the maximum concurrent connections at runtime |
| `setMaxTasksInFlight(n)` | Change the maximum in-flight tasks at runtime |
| `setPolicy(policy)` | Switch the resource policy at runtime |
| `flush()` | Wait for the internal command queue to drain (useful before shutdown) |
| `shutdown()` | Shut down the downloader; unfinished tasks fail immediately (`ERR_SHUTDOWN`) and the instance cannot be reused |

### Example: runtime tuning and graceful exit

```ts
// Limit concurrency during the day, open it up at night
downloader.setMaxConcurrency(4);
downloader.setPolicy('favor-other-apps');

// Graceful shutdown: drain the queue before exiting
process.on('SIGINT', async () => {
  await downloader.flush();
  downloader.shutdown();
});
```

## Static convenience methods

```ts
// One-shot download; the instance shuts down automatically afterwards
static Downloader.download(url, destination, options?): Promise<ProgressEvent>

// Probe URL metadata without downloading content
static Downloader.probe(url, options?): Promise<ProbeResult>
```

`download` accepts partial `DownloadTaskOptions` fields via `options.task` (such as `checksum`, `headers`); `probe` accepts `options.timeoutMs`, which defaults to 30000.

### Example: probe first, then decide whether to download

```ts
const info = await Downloader.probe('https://example.com/movie.mkv');
console.log(info.contentLength, info.acceptRanges); // size and range support

if (info.ok && info.contentLength > 500 * 1024 * 1024) {
  await Downloader.download('https://example.com/movie.mkv', 'D:/Downloads/movie.mkv');
}
```

### Example: one-shot download with a checksum

```ts
await Downloader.download(
  'https://example.com/tool.zip',
  'D:/Downloads/tool.zip',
  {
    engine: { maxConcurrency: 4 },
    task: {
      checksum: { algorithm: 'sha256', value: 'e3b0c44298fc1c14...' },
      overwrite: 'rename',
    },
  },
);
```

### ProbeResult

| Field | Description |
| --- | --- |
| `ok` | Whether the probe succeeded |
| `statusCode` | HTTP status code |
| `contentLength` | File size (-1 when unknown) |
| `acceptRanges` | Whether the server supports Range segmentation |
| `finalUrl` | Final URL after redirects |
| `etag` / `lastModified` / `contentType` | Response header info |
| `message` | Additional message |

## Stats

| Field | Description |
| --- | --- |
| `activeTasks` / `queuedTasks` | Active / queued task counts |
| `effectiveConcurrency` / `maxConcurrency` | Current effective concurrency / limit |
| `policy` | Current resource policy |
| `cpuUsagePct` / `memAvailablePct` / `diskWriteLatencyMs` | System resource metrics (used by adaptive scheduling) |
| `totalSpeedBps` | Global download speed (B/s) |
| `totalDownloaded` / `totalRemaining` | Total bytes downloaded / remaining |
| `degradedTasks` | Number of tasks in degraded mode |

### Example: printing global stats periodically

```ts
downloader.on('stats', (s) => {
  console.log(
    `[global] active ${s.activeTasks} queued ${s.queuedTasks} | ` +
    `${(s.totalSpeedBps / 1e6).toFixed(1)} MB/s | ` +
    `CPU ${s.cpuUsagePct.toFixed(0)}%`
  );
});
```

## Events

| Event | Arguments | Description |
| --- | --- | --- |
| `task` | `DownloadTask` | A new task was added |
| `progress` | `(task, ProgressEvent)` | Progress updated |
| `state` | `(task, ProgressEvent)` | Task state changed |
| `degraded` | `(task, DegradedEvent)` | Task entered degraded mode |
| `reset` | `(task, ProgressEvent)` | Task progress was reset (e.g. re-downloading after a validator change) |
| `completed` | `(task, ProgressEvent)` | Task completed |
| `failed` | `(task, DownloadError, ProgressEvent)` | Task failed |
| `stats` | `StatsEvent` | Periodic global statistics |
| `log` | `LogEvent` | Engine log (`level`, `scope`, `message`) |
| `warning` | `Error` | Non-fatal internal error |
| `close` | - | The downloader has closed |

### Example: complete instance usage

```ts
import { Downloader, DownloadError } from 'win-down';

const downloader = new Downloader({
  maxConcurrency: 8,
  resume: true,
  policy: 'balanced',
});

// Global listeners: shared by all tasks
downloader.on('completed', (task) => {
  console.log(`Completed: ${task.destination} (checksum ${task.checksumVerified ? 'verified' : 'not configured'})`);
});
downloader.on('failed', (task, err: DownloadError) => {
  console.error(`Failed: ${task.url} -> ${err.code}: ${err.message}`);
});
downloader.on('degraded', (task, e) => {
  console.warn(`Degraded: ${task.url}, reason: ${e.degradeReason}`);
});

const task = downloader.addTask({
  url: 'https://example.com/file.iso',
  destination: 'D:/Downloads/file.iso',
});

try {
  const result = await task.wait();
  console.log(`Elapsed ${(result.elapsedMs / 1000).toFixed(1)}s`);
} catch (err) {
  // failures are also thrown here as DownloadError
}

downloader.shutdown();
```

## Resource policies

| Policy | Description |
| --- | --- |
| `balanced` | Default; balances speed against system load |
| `favor-other-apps` | Limits its own concurrency and throughput, yielding resources to foreground apps |
| `max-performance` | Maximizes throughput, allowing higher CPU/disk usage |

The engine adaptively adjusts the effective concurrency within policy constraints based on `cpuUsagePct`, `memAvailablePct`, and `diskWriteLatencyMs`.

### Example: background downloads

```ts
// Download large files in the background with minimal impact on the user
const downloader = new Downloader({
  policy: 'favor-other-apps',
  maxConcurrency: 4,
  maxTasksInFlight: 2,
});
```

## Related docs

- [DownloadTask API](task.md)
- [Error codes and degradation reasons](errors.md)
