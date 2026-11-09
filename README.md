# win-down

**English** | [简体中文](https://github.com/zhuxiaojt/win-down/blob/main/README-CN.md)

A multi-threaded downloader for Windows. The core engine is implemented in Rust + WinHTTP and exposed to Node.js via Node-API (N-API), with full TypeScript type support.

> Windows only (depends on WinHTTP and Win32 APIs).

## Features

- **Multi-threaded segmented downloads**: automatically detects `Range` support and downloads in parallel segments
- **Adaptive degradation**: automatically falls back to a single connection when the server doesn't support segmentation, and reports the reason
- **Resumable downloads**: resumes based on segment progress and re-downloads from scratch when validators change
- **Mirror failover**: configure multiple mirror URLs that are tried automatically on failure
- **Retry with backoff**: configurable exponential backoff
- **Integrity verification**: SHA-256 / MD5 / CRC32
- **Concurrency scheduling**: priority queue + adaptive concurrency based on system resources
- **Resource policies**: `balanced` / `favor-other-apps` / `max-performance`
- **Proxy support**: uses the system proxy by default, or an explicit one
- **Event-driven**: progress, state, degradation, stats, and log events are all pushed via EventEmitter

## Install

Requires Node.js >= 18, and Windows.

```bash
npm install win-down
```

## Quick Start

```ts
import { Downloader } from 'win-down';

// One-shot download
await Downloader.download('https://example.com/file.zip', 'D:/Downloads/file.zip');

// Or use an instance to manage multiple tasks
const downloader = new Downloader({ maxConcurrency: 8, resume: true });

const task = downloader.addTask({
  url: 'https://example.com/file.zip',
  destination: 'D:/Downloads/file.zip',
  checksum: { algorithm: 'sha256', value: '...' },
});

task.on('progress', (e) => {
  console.log(`${e.percent.toFixed(1)}%  ${(e.speedBps / 1e6).toFixed(1)} MB/s`);
});

await task.wait(); // resolves on completion, rejects with DownloadError on failure

downloader.shutdown();
```

CJS works too:

```js
const { Downloader } = require('win-down');
```

## Documentation

For detailed API documentation, see [`docs/en/`](https://github.com/zhuxiaojt/win-down/blob/main/docs/en/):

| Document | Contents |
| --- | --- |
| [Downloader API](https://github.com/zhuxiaojt/win-down/blob/main/docs/en/downloader.md) | Downloader instance, all options, global stats, events, resource policies |
| [DownloadTask API](https://github.com/zhuxiaojt/win-down/blob/main/docs/en/task.md) | Task options, properties, methods, events, progress event fields, resume |
| [Error codes & degradation](https://github.com/zhuxiaojt/win-down/blob/main/docs/en/errors.md) | DownloadError codes, DegradeReason, task state machine |

## Architecture

```
src/                  # TypeScript binding layer (CJS implementation + ESM re-export)
native/               # Rust core engine (WinHTTP / Win32 / BCrypt)
  src/engine/         #   scheduler, workers, transfer, resume, monitor, lifecycle
scripts/build.js      # cargo build + copy of the .node module
test/                 # local HTTP server + integration tests
```

## License

MIT
