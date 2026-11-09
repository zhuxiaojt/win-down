# Error codes, degradation reasons, and states

## DownloadError

All failures (`task.wait()` rejection, `failed` events) raise/propagate a `DownloadError`:

```ts
class DownloadError extends Error {
  code: string; // error code
}
```

### Example: uniform error handling

```ts
import { DownloadError } from 'win-down';

try {
  await task.wait();
} catch (err) {
  if (err instanceof DownloadError) {
    switch (err.code) {
      case 'ERR_FILE_EXISTS':
        console.log('File already exists; retry with overwrite: rename');
        break;
      case 'ERR_CHECKSUM_MISMATCH':
        console.log('Downloaded content is corrupted or tampered with; download again');
        break;
      case 'ERR_RETRY_EXHAUSTED':
        console.log('Network is unstable; retry limit reached');
        break;
      default:
        console.log(`Download failed ${err.code}: ${err.message}`);
    }
  } else {
    throw err; // not a download error; rethrow
  }
}
```

### Example: handling failures via events

```ts
downloader.on('failed', (task, err: DownloadError) => {
  // err.code and err.message are available here too
  log.error(`Task ${task.id} failed: ${err.code}`);
});
```

## Error codes

| Code | Source | Description |
| --- | --- | --- |
| `ERR_FILE_EXISTS` | Task | Destination exists and `overwrite: 'error'` is set |
| `ERR_HTTP_STATUS` | Task | Bad HTTP status code (e.g. 404, 403, 5xx) |
| `ERR_PROBE_FAILED` | Task | URL probe failed |
| `ERR_RETRY_EXHAUSTED` | Task | Retry limit exhausted |
| `ERR_IO` | Task | Local file read/write failure |
| `ERR_SIZE_MISMATCH` | Task | Actual size differs from `expectedSize` |
| `ERR_CHECKSUM_MISMATCH` | Task | Checksum mismatch |
| `ERR_CHECKSUM` | Task | Checksum computation error |
| `ERR_CONTENT_ENCODING` | Task | Response is compressed (`Content-Encoding`); segmentation/resume is not possible |
| `ERR_CANCELLED` | Binding | Task was cancelled |
| `ERR_CLOSED` | Binding | A method was called after the downloader was closed |
| `ERR_SHUTDOWN` | Binding | Task failed because the downloader shut down |
| `ERR_PROBE_TIMEOUT` | Binding | Probe timed out |
| `ERR_NOT_FOUND` | Engine command | The task being operated on does not exist |
| `ERR_TASK_TERMINAL` | Engine command | Task is already terminal; cannot pause/resume |
| `ERR_TASK_ACTIVE` | Engine command | Task is still running; cannot remove |
| `ERR_INVALID_ARG` | Engine command | Invalid argument/command |
| `ERR_UNKNOWN` | Binding | Unknown error |

> Binding-layer exceptions (e.g. invalid `addTask` arguments) throw directly; runtime failures are delivered via the `failed` event and `task.wait()` rejection.

## Degradation reasons (DegradeReason)

When segmented downloading is not applicable, the engine automatically degrades to a single connection and emits a `degraded` event. Degradation is not an error — the task still completes normally:

| Reason | Description |
| --- | --- |
| `no-range-support` | Server does not support `Range` requests |
| `unknown-length` | File size unknown (no `Content-Length`) |
| `small-file` | File is smaller than `smallFileThreshold`; segmentation unnecessary |
| `range-ignored` | Server ignores the `Range` header and returns the full content |
| `range-mismatch` | The `Range` response does not match the requested range |
| `content-encoding` | Response carries `Content-Encoding`, conflicting with Range semantics |
| `validator-changed` | ETag / Last-Modified changed; resume abandoned and download restarted |
| `remote-volume` | Server/network limits concurrency (conservative degradation) |
| `server-error-storm` | Server keeps returning errors; consolidated into a single connection |

### Example: observing degradation and notifying the user

```ts
task.on('degraded', (e) => {
  if (e.degradeReason === 'no-range-support') {
    console.log('Server does not support segmentation; switched to a single connection, so it may be slower');
  } else if (e.degradeReason === 'validator-changed') {
    console.log('Remote file changed; resume abandoned, restarting from scratch');
  }
});
```

## Task states (TaskState)

| State | Description |
| --- | --- |
| `queued` | Waiting in queue |
| `probing` | Probing the URL |
| `downloading` | Downloading |
| `retrying` | Waiting to retry |
| `paused` | Paused |
| `verifying` | Verifying |
| `completed` | Completed (terminal) |
| `failed` | Failed (terminal) |
| `cancelled` | Cancelled (terminal) |

### Example: rendering UI based on state

```ts
switch (task.state) {
  case 'queued':    text = 'Queued...'; break;
  case 'downloading': text = `${task.percent?.toFixed(0)}%`; break;
  case 'retrying':  text = `Retry #${task.attempts}...`; break;
  case 'paused':    text = 'Paused'; break;
  case 'completed': text = task.checksumVerified ? 'Done (verified)' : 'Done'; break;
  case 'failed':    text = `Failed: ${task.error?.code}`; break;
}
```

## Related docs

- [Downloader API](downloader.md)
- [DownloadTask API](task.md)
