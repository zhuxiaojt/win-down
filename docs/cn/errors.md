# 错误码、降级原因与状态

## DownloadError

所有失败（`task.wait()` reject、`failed` 事件）均抛出/传递 `DownloadError`：

```ts
class DownloadError extends Error {
  code: string; // 错误码
}
```

### 示例：统一处理错误

```ts
import { DownloadError } from 'win-down';

try {
  await task.wait();
} catch (err) {
  if (err instanceof DownloadError) {
    switch (err.code) {
      case 'ERR_FILE_EXISTS':
        console.log('文件已存在，改用 overwrite: rename 重试');
        break;
      case 'ERR_CHECKSUM_MISMATCH':
        console.log('下载内容被篡改或传输损坏，请重新下载');
        break;
      case 'ERR_RETRY_EXHAUSTED':
        console.log('网络不稳定，重试次数已用尽');
        break;
      default:
        console.log(`下载失败 ${err.code}: ${err.message}`);
    }
  } else {
    throw err; // 非下载错误，继续抛出
  }
}
```

### 示例：通过事件处理失败

```ts
downloader.on('failed', (task, err: DownloadError) => {
  // err.code 与 err.message 同样可用
  log.error(`任务 ${task.id} 失败: ${err.code}`);
});
```

## 错误码

| 错误码 | 来源 | 说明 |
| --- | --- | --- |
| `ERR_FILE_EXISTS` | 任务 | 目标文件已存在且 `overwrite: 'error'` |
| `ERR_HTTP_STATUS` | 任务 | HTTP 状态码错误（如 404、403、5xx） |
| `ERR_PROBE_FAILED` | 任务 | URL 探测失败 |
| `ERR_RETRY_EXHAUSTED` | 任务 | 重试次数用尽 |
| `ERR_IO` | 任务 | 本地文件读写失败 |
| `ERR_SIZE_MISMATCH` | 任务 | 实际大小与 `expectedSize` 不符 |
| `ERR_CHECKSUM_MISMATCH` | 任务 | 校验和不匹配 |
| `ERR_CHECKSUM` | 任务 | 校验和计算错误 |
| `ERR_CONTENT_ENCODING` | 任务 | 响应被压缩（`Content-Encoding`），无法分片/续传 |
| `ERR_CANCELLED` | 绑定层 | 任务被取消 |
| `ERR_CLOSED` | 绑定层 | 下载器已关闭后仍调用方法 |
| `ERR_SHUTDOWN` | 绑定层 | 下载器 `shutdown()` 导致任务失败 |
| `ERR_PROBE_TIMEOUT` | 绑定层 | 探测超时 |
| `ERR_NOT_FOUND` | 引擎命令 | 操作的任务不存在 |
| `ERR_TASK_TERMINAL` | 引擎命令 | 任务已是终态，无法暂停/恢复 |
| `ERR_TASK_ACTIVE` | 引擎命令 | 任务仍在运行，无法移除 |
| `ERR_INVALID_ARG` | 引擎命令 | 无效参数/命令 |
| `ERR_UNKNOWN` | 绑定层 | 未知错误 |

> 绑定层异常（如 `addTask` 参数非法）会直接抛出；运行中失败通过 `failed` 事件与 `task.wait()` reject 传递。

## 降级原因（DegradeReason）

分片下载不适用时，引擎自动降级为单连接下载并发出 `degraded` 事件。降级不是错误——任务仍会正常完成：

| 原因 | 说明 |
| --- | --- |
| `no-range-support` | 服务器不支持 `Range` 请求 |
| `unknown-length` | 文件大小未知（无 `Content-Length`） |
| `small-file` | 文件小于 `smallFileThreshold`，无需切片 |
| `range-ignored` | 服务器忽略 `Range` 头，返回完整内容 |
| `range-mismatch` | `Range` 响应与请求的范围不一致 |
| `content-encoding` | 响应带 `Content-Encoding`，内容与 Range 语义冲突 |
| `validator-changed` | ETag / Last-Modified 变化，放弃续传从头下载 |
| `remote-volume` | 服务器/网络限制并发（保守降级） |
| `server-error-storm` | 服务器连续返回错误，收敛为单连接 |

### 示例：观测降级并提示用户

```ts
task.on('degraded', (e) => {
  if (e.degradeReason === 'no-range-support') {
    console.log('服务器不支持分片，速度会慢一些，已切换为单连接下载');
  } else if (e.degradeReason === 'validator-changed') {
    console.log('远端文件已更新，放弃续传，从头开始下载');
  }
});
```

## 任务状态（TaskState）

| 状态 | 说明 |
| --- | --- |
| `queued` | 排队中 |
| `probing` | 探测 URL |
| `downloading` | 下载中 |
| `retrying` | 等待重试 |
| `paused` | 已暂停 |
| `verifying` | 校验中 |
| `completed` | 完成（终态） |
| `failed` | 失败（终态） |
| `cancelled` | 取消（终态） |

### 示例：根据状态渲染 UI

```ts
switch (task.state) {
  case 'queued':    text = '排队中...'; break;
  case 'downloading': text = `${task.percent?.toFixed(0)}%`; break;
  case 'retrying':  text = `第 ${task.attempts} 次重试...`; break;
  case 'paused':    text = '已暂停'; break;
  case 'completed': text = task.checksumVerified ? '完成 (校验通过)' : '完成'; break;
  case 'failed':    text = `失败: ${task.error?.code}`; break;
}
```

## 相关文档

- [Downloader API](downloader.md)
- [DownloadTask API](task.md)
