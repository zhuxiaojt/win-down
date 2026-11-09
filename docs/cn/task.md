# DownloadTask API

`DownloadTask` 表示一个下载任务，继承自 `EventEmitter`，由 `Downloader.addTask()` 创建。

```ts
const task = downloader.addTask({
  url: 'https://example.com/file.zip',
  destination: 'D:/Downloads/file.zip',
  checksum: { algorithm: 'sha256', value: 'abc...' },
});

task.on('progress', (e) => console.log(e.percent));

await task.wait(); // 完成 → resolve(ProgressEvent)；失败/取消 → reject(DownloadError)
```

## DownloadTaskOptions

| 选项 | 类型 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `url` | `string` | （必填） | 下载地址 |
| `destination` | `string` | （必填） | 目标文件路径 |
| `mirrors` | `string[]` | `[]` | 镜像地址列表，主 URL 失败时依次切换 |
| `headers` | `Record<string, string>` | - | 任务级请求头（禁止 CR/LF） |
| `expectedSize` | `number` | `-1` | 期望文件大小，下载完成后校验（-1 表示不校验） |
| `checksum` | `{ algorithm, value }` | - | 完整性校验；`algorithm` 为 `'sha256' \| 'md5' \| 'crc32'`，`value` 为十六进制摘要 |
| `priority` | `number` | `0` | 队列优先级，数值越大越先调度 |
| `maxConcurrency` | `number` | 全局值 | 任务级并发连接数覆盖 |
| `overwrite` | `'error' \| 'overwrite' \| 'rename' \| 'skip'` | `'error'` | 目标文件已存在时的处理方式 |
| `resume` | `boolean` | 全局值 | 任务级断点续传开关 |
| `segmentSize` | `number` | 全局值 | 任务级分片大小覆盖 |

### overwrite 模式

| 模式 | 行为 |
| --- | --- |
| `error` | 已存在时任务失败（`ERR_FILE_EXISTS`） |
| `overwrite` | 直接覆盖已有文件 |
| `rename` | 自动重命名（如 `file (1).zip`） |
| `skip` | 跳过下载，任务直接标记完成（`skipped` 为 `true`） |

### 示例：完整配置的任务

```ts
const task = downloader.addTask({
  url: 'https://mirror1.example.com/app-1.0.zip',
  // 主源失败时自动尝试镜像
  mirrors: [
    'https://mirror2.example.com/app-1.0.zip',
    'https://mirror3.example.com/app-1.0.zip',
  ],
  destination: 'D:/Downloads/app-1.0.zip',
  headers: { Authorization: 'Bearer <token>' },
  expectedSize: 104857600, // 下载完校验实际大小
  checksum: { algorithm: 'sha256', value: 'e3b0c44298fc1c14...' },
  priority: 10,            // 高优先级，先于普通任务调度
  maxConcurrency: 16,      // 该任务单独放开并发
  overwrite: 'rename',     // 已存在则重命名
  resume: true,            // 开启断点续传
});
```

### 示例：skip 与 rename 的区别

```ts
// 已存在 D:/Downloads/report.pdf 时：
// error（默认）→ 任务失败，错误码 ERR_FILE_EXISTS
// rename       → 下载为 report (1).pdf
// skip         → 不下载，直接完成，task.skipped === true

const t = downloader.addTask({
  url: 'https://example.com/report.pdf',
  destination: 'D:/Downloads/report.pdf',
  overwrite: 'skip',
});
await t.wait();
console.log(t.skipped); // true（如果文件本来就存在）
```

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `id` | `string` | 任务 ID |
| `url` / `destination` | `string` | 创建时传入的地址与目标路径 |
| `state` | `TaskState` | 当前状态，见下表 |
| `done` | `boolean`（getter） | 是否处于终态 |
| `totalBytes` | `number \| null` | 文件总大小（未知为 `null`） |
| `downloadedBytes` | `number` | 已下载字节（含历史会话） |
| `percent` | `number \| null` | 进度百分比（未知为 `null`） |
| `speedBps` | `number` | 瞬时速度（B/s） |
| `etaSeconds` | `number \| null` | 预计剩余时间（秒） |
| `activeConnections` | `number` | 当前活跃连接数 |
| `segmentsTotal` / `segmentsCompleted` | `number` | 分片总数 / 已完成分片数 |
| `degraded` | `boolean` | 是否处于降级模式 |
| `degradeReason` | `DegradeReason \| null` | 降级原因，见 [errors.md](errors.md) |
| `checksumVerified` | `boolean \| null` | 校验结果（未配置校验时为 `null`） |
| `skipped` | `boolean` | 因 `overwrite: 'skip'` 跳过 |
| `attempts` | `number` | 已重试次数 |
| `elapsedMs` | `number` | 任务耗时（毫秒） |
| `resumedBytes` | `number` | 从断点恢复的字节数 |
| `sessionBytes` | `number` | 本次会话下载的字节数 |
| `error` | `DownloadError \| null` | 失败时的错误对象 |
| `finalUrl` / `etag` / `lastModified` / `contentType` | `string \| null` | 探测/响应得到的元信息 |
| `downloader` | `Downloader` | 所属下载器实例 |

### 示例：实时渲染进度条

```ts
task.on('progress', (e) => {
  const bar = '█'.repeat(Math.floor(e.percent / 5)) + '░'.repeat(20 - Math.floor(e.percent / 5));
  process.stdout.write(
    `\r${bar} ${e.percent.toFixed(1)}%  ` +
    `${(e.downloadedBytes / 1e6).toFixed(0)}/${((e.totalBytes ?? 0) / 1e6).toFixed(0)}MB  ` +
    `${(e.speedBps / 1e6).toFixed(1)}MB/s  ETA ${e.etaSeconds}s  ` +
    `连接 ${e.activeConnections} 分片 ${e.segmentsCompleted}/${e.segmentsTotal} `
  );
});
```

### 示例：读取任务终态信息

```ts
await task.wait();
console.log({
  state: task.state,              // 'completed'
  checksumVerified: task.checksumVerified, // true
  elapsedMs: task.elapsedMs,      // 总耗时
  resumedBytes: task.resumedBytes, // 续传恢复的字节数（0 表示全新下载）
  finalUrl: task.finalUrl,        // 重定向后的最终地址
});
```

## TaskState

状态机：`queued → probing → downloading ⇄ retrying → verifying → completed`，任何阶段可进入 `paused` / `failed` / `cancelled`。

| 状态 | 说明 |
| --- | --- |
| `queued` | 排队中 |
| `probing` | 探测 URL 元信息 |
| `downloading` | 下载中 |
| `retrying` | 等待重试 |
| `paused` | 已暂停 |
| `verifying` | 校验大小/校验和 |
| `completed` | 完成（终态） |
| `failed` | 失败（终态） |
| `cancelled` | 已取消（终态） |

### 示例：监听状态变化

```ts
task.on('state', (e) => {
  if (e.state === 'paused') console.log('已暂停，进度已保存');
  if (e.state === 'verifying') console.log('下载完成，正在校验...');
});
```

## 方法

| 方法 | 说明 |
| --- | --- |
| `wait(): Promise<ProgressEvent>` | 等待任务结束；完成时 resolve 最终进度事件，失败/取消时 reject `DownloadError`。可重复调用 |
| `pause()` / `resume()` | 暂停 / 恢复 |
| `cancel({ removePartialFile? }?)` | 取消任务，可选删除未完成片段 |
| `on(event, listener)` | 监听事件（带完整类型重载） |

### 示例：wait 的三种结果

```ts
// 1. 正常完成
const result = await task.wait();

// 2. 失败
try { await failingTask.wait(); }
catch (err) { console.log(err.code, err.message); }

// 3. 取消
try { await cancelledTask.wait(); }
catch (err) { console.log(err.code); } // 'ERR_CANCELLED'
```

### 示例：超时自动取消

```ts
const task = downloader.addTask({ url, destination });

const timer = setTimeout(() => task.cancel({ removePartialFile: true }), 10 * 60_000);
try {
  await task.wait();
} finally {
  clearTimeout(timer);
}
```

## 事件

| 事件 | 参数 | 说明 |
| --- | --- | --- |
| `progress` | `ProgressEvent` | 进度更新 |
| `state` | `ProgressEvent` | 状态变化 |
| `degraded` | `DegradedEvent` | 进入降级模式 |
| `reset` | `ProgressEvent` | 进度重置（如校验器变化后从头重下） |
| `completed` | `ProgressEvent` | 完成 |
| `failed` | `(DownloadError, ProgressEvent)` | 失败 |
| `cancelled` | - | 已取消 |

## ProgressEvent

进度与结果事件统一携带以下字段：

| 字段 | 说明 |
| --- | --- |
| `kind` | `'progress' \| 'state' \| 'reset' \| 'completed' \| 'failed'` |
| `taskId` / `url` / `destination` | 任务标识 |
| `state` | 当前 `TaskState` |
| `totalBytes` / `downloadedBytes` | 总大小 / 已下载 |
| `sessionBytes` / `resumedBytes` | 本次会话字节 / 恢复字节 |
| `percent` | 百分比（未知为 -1） |
| `speedBps` / `averageSpeedBps` | 瞬时 / 平均速度 |
| `etaSeconds` | 预计剩余秒数 |
| `activeConnections` | 活跃连接数 |
| `segmentsTotal` / `segmentsCompleted` | 分片进度 |
| `degraded` / `degradeReason` | 降级标记与原因 |
| `elapsedMs` / `attempts` / `connectionsUsed` | 耗时 / 重试次数 / 累计使用的连接数 |
| `checksumVerified` / `skipped` | 校验结果 / 是否跳过 |
| `finalUrl` / `etag` / `lastModified` / `contentType` | 响应元信息 |
| `code` / `message` | 失败时的错误码与消息 |

## 断点续传

开启 `resume`（全局或任务级）后：

- 任务暂停或进程退出后重新添加同 URL 同目标的任务，会基于已下载字节与分片进度续传
- 服务器 ETag / Last-Modified 发生变化时自动放弃续传并从头重下（触发 `reset` 事件，降级原因 `validator-changed`）
- `resumedBytes` 字段可观察实际恢复的字节数

### 示例：进程重启后续传

```ts
// 第一次运行：下载到一半被 Ctrl+C 中断
// 第二次运行：同样的 url + destination，自动从断点继续
const task = downloader.addTask({
  url: 'https://example.com/big.iso',
  destination: 'D:/Downloads/big.iso',
  resume: true,
});
task.on('progress', (e) => {
  if (e.resumedBytes > 0) console.log(`已从断点恢复 ${(e.resumedBytes / 1e6).toFixed(0)}MB`);
});
await task.wait();
```

## 相关文档

- [Downloader API](downloader.md)
- [错误码与降级原因](errors.md)
