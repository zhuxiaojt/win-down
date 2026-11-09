# Downloader API

`Downloader` 是核心入口，管理任务队列、并发调度与全局统计。继承自 `EventEmitter`。

```ts
import { Downloader } from 'win-down';

const downloader = new Downloader({
  maxConcurrency: 8,
  resume: true,
});
```

## 构造函数

```ts
new Downloader(options?: DownloaderOptions)
```

### DownloaderOptions

| 选项 | 类型 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `maxConcurrency` | `number` | `16` | 全局最大并发连接数（1–256） |
| `maxTasksInFlight` | `number` | `4` | 最大同时下载任务数（1–64） |
| `policy` | `'balanced' \| 'favor-other-apps' \| 'max-performance'` | `'balanced'` | 资源策略，见[资源策略](#资源策略) |
| `segmentSize` | `number` | `8 MiB` | 分片大小（字节） |
| `maxSegmentsPerTask` | `number` | `32` | 每任务最大分片数（1–1024） |
| `smallFileThreshold` | `number` | `2 MiB` | 小于该值的文件不切片，直接单连接下载 |
| `maxPerHost` | `number` | `6` | 每主机最大连接数 |
| `connectTimeoutMs` | `number` | `10000` | 连接超时（毫秒） |
| `idleTimeoutMs` | `number` | `30000` | 空闲超时（毫秒） |
| `maxRetries` | `number` | `5` | 最大重试次数 |
| `baseRetryMs` | `number` | `250` | 重试退避基础间隔（毫秒） |
| `maxRetryMs` | `number` | `30000` | 重试退避上限（毫秒） |
| `followRedirects` | `boolean` | `true` | 是否跟随重定向 |
| `maxRedirects` | `number` | `10` | 最大重定向次数 |
| `verifyTls` | `boolean` | `true` | 是否校验 TLS 证书 |
| `proxy` | `string` | `'system'` | 代理地址；默认使用系统代理 |
| `userAgent` | `string` | `'win-down/1.0'` | User-Agent |
| `headers` | `Record<string, string>` | - | 全局默认请求头（禁止 CR/LF） |
| `resume` | `boolean` | `false` | 全局断点续传开关 |
| `preallocate` | `boolean` | `true` | 预分配文件空间 |
| `progressIntervalMs` | `number` | `100` | 进度事件推送间隔（最小 20ms） |
| `ioBufferSize` | `number` | `512 KiB` | I/O 缓冲区大小（64 KiB – 4 MiB） |
| `createDirectories` | `boolean` | `true` | 自动创建目标目录 |

## 任务管理

| 方法 | 返回 | 说明 |
| --- | --- | --- |
| `addTask(options: DownloadTaskOptions)` | `DownloadTask` | 添加下载任务。下载器关闭后抛出 `ERR_CLOSED` |
| `getTask(id: string)` | `DownloadTask \| null` | 按 ID 查找任务 |
| `get tasks` | `DownloadTask[]` | 当前所有任务快照 |

### 示例：添加任务与查询

```ts
const task = downloader.addTask({
  url: 'https://example.com/installer.exe',
  destination: 'D:/Downloads/installer.exe',
});

console.log(task.id);            // 任务 ID，可用于 getTask
console.log(downloader.tasks.length); // 当前任务总数
```

## 任务控制

| 方法 | 说明 |
| --- | --- |
| `pause(id)` | 暂停任务 |
| `resume(id)` | 恢复任务 |
| `cancel(id, { removePartialFile? })` | 取消任务；`removePartialFile` 为 `true` 时同时删除未完成的文件片段 |
| `pauseAll()` | 暂停所有任务 |
| `resumeAll()` | 恢复所有任务 |
| `cancelAll({ removePartialFile? })` | 取消所有任务 |
| `removeTask(id, { removePartialFile? })` | 取消并从任务列表中移除记录 |
| `clearCompleted()` | 清除所有已完成/失败/取消的任务记录 |

### 示例：批量任务与暂停/恢复

```ts
const urls = [
  'https://example.com/a.zip',
  'https://example.com/b.zip',
  'https://example.com/c.zip',
];

for (const url of urls) {
  downloader.addTask({ url, destination: `D:/Downloads/${url.split('/').pop()}` });
}

// 所有任务排队后，暂停全部
await downloader.pauseAll();

// ...稍后恢复
await downloader.resumeAll();

// 清理已完成任务的记录
await downloader.clearCompleted();
```

## 全局控制

| 方法 | 说明 |
| --- | --- |
| `getStats(): Stats` | 获取全局统计快照 |
| `setMaxConcurrency(n)` | 运行时修改最大并发连接数 |
| `setMaxTasksInFlight(n)` | 运行时修改最大同时任务数 |
| `setPolicy(policy)` | 运行时切换资源策略 |
| `flush()` | 等待内部命令队列全部处理完成（常用于关闭前确认） |
| `shutdown()` | 关闭下载器；未完成任务立即标记为失败（`ERR_SHUTDOWN`），实例不可复用 |

### 示例：运行时调整与优雅退出

```ts
// 白天限制并发，晚上放开
downloader.setMaxConcurrency(4);
downloader.setPolicy('favor-other-apps');

// 优雅关闭：等待队列处理完再退出
process.on('SIGINT', async () => {
  await downloader.flush();
  downloader.shutdown();
});
```

## 静态便捷方法

```ts
// 一次性下载，完成后自动 shutdown
static Downloader.download(url, destination, options?): Promise<ProgressEvent>

// 探测 URL 元信息，不下载内容
static Downloader.probe(url, options?): Promise<ProbeResult>
```

`download` 的 `options.task` 可传入 `DownloadTaskOptions` 的部分字段（如 `checksum`、`headers`）；`probe` 的 `options.timeoutMs` 默认 30000。

### 示例：探测后再决定是否下载

```ts
const info = await Downloader.probe('https://example.com/movie.mkv');
console.log(info.contentLength, info.acceptRanges); // 大小、是否支持分片

if (info.ok && info.contentLength > 500 * 1024 * 1024) {
  await Downloader.download('https://example.com/movie.mkv', 'D:/Downloads/movie.mkv');
}
```

### 示例：带校验和的一次性下载

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

| 字段 | 说明 |
| --- | --- |
| `ok` | 探测是否成功 |
| `statusCode` | HTTP 状态码 |
| `contentLength` | 文件大小（未知为 -1） |
| `acceptRanges` | 服务器是否支持 Range 分片 |
| `finalUrl` | 重定向后的最终 URL |
| `etag` / `lastModified` / `contentType` | 响应头信息 |
| `message` | 附加消息 |

## Stats

| 字段 | 说明 |
| --- | --- |
| `activeTasks` / `queuedTasks` | 进行中 / 排队任务数 |
| `effectiveConcurrency` / `maxConcurrency` | 当前生效并发数 / 上限 |
| `policy` | 当前资源策略 |
| `cpuUsagePct` / `memAvailablePct` / `diskWriteLatencyMs` | 系统资源指标（用于自适应调度） |
| `totalSpeedBps` | 全局下载速度（B/s） |
| `totalDownloaded` / `totalRemaining` | 累计已下载 / 剩余字节 |
| `degradedTasks` | 处于降级模式的任务数 |

### 示例：周期性打印全局统计

```ts
downloader.on('stats', (s) => {
  console.log(
    `[全局] 活跃 ${s.activeTasks} 排队 ${s.queuedTasks} | ` +
    `${(s.totalSpeedBps / 1e6).toFixed(1)} MB/s | ` +
    `CPU ${s.cpuUsagePct.toFixed(0)}%`
  );
});
```

## 事件

| 事件 | 参数 | 说明 |
| --- | --- | --- |
| `task` | `DownloadTask` | 新任务加入 |
| `progress` | `(task, ProgressEvent)` | 进度更新 |
| `state` | `(task, ProgressEvent)` | 任务状态变化 |
| `degraded` | `(task, DegradedEvent)` | 任务进入降级模式 |
| `reset` | `(task, ProgressEvent)` | 任务下载进度被重置（如校验器变化后重下） |
| `completed` | `(task, ProgressEvent)` | 任务完成 |
| `failed` | `(task, DownloadError, ProgressEvent)` | 任务失败 |
| `stats` | `StatsEvent` | 周期性全局统计 |
| `log` | `LogEvent` | 引擎日志（`level`、`scope`、`message`） |
| `warning` | `Error` | 非致命内部错误 |
| `close` | - | 下载器已关闭 |

### 示例：完整的实例使用流程

```ts
import { Downloader, DownloadError } from 'win-down';

const downloader = new Downloader({
  maxConcurrency: 8,
  resume: true,
  policy: 'balanced',
});

// 全局监听：所有任务共用
downloader.on('completed', (task) => {
  console.log(`完成: ${task.destination} (校验${task.checksumVerified ? '通过' : '未配置'})`);
});
downloader.on('failed', (task, err: DownloadError) => {
  console.error(`失败: ${task.url} -> ${err.code}: ${err.message}`);
});
downloader.on('degraded', (task, e) => {
  console.warn(`降级: ${task.url}, 原因: ${e.degradeReason}`);
});

const task = downloader.addTask({
  url: 'https://example.com/file.iso',
  destination: 'D:/Downloads/file.iso',
});

try {
  const result = await task.wait();
  console.log(`总耗时 ${(result.elapsedMs / 1000).toFixed(1)}s`);
} catch (err) {
  // 失败会在这里抛出 DownloadError
}

downloader.shutdown();
```

## 资源策略

| 策略 | 说明 |
| --- | --- |
| `balanced` | 默认，兼顾速度与系统负载 |
| `favor-other-apps` | 限制自身并发与吞吐，让出资源给前台应用 |
| `max-performance` | 最大化吞吐，允许更高 CPU/磁盘占用 |

引擎会根据 `cpuUsagePct`、`memAvailablePct`、`diskWriteLatencyMs` 在策略约束内自适应调节生效并发数。

### 示例：后台挂机下载

```ts
// 挂机下载大文件，尽量不影响用户使用电脑
const downloader = new Downloader({
  policy: 'favor-other-apps',
  maxConcurrency: 4,
  maxTasksInFlight: 2,
});
```

## 相关文档

- [DownloadTask API](task.md)
- [错误码与降级原因](errors.md)
