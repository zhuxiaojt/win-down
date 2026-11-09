# win-down

[English](https://github.com/zhuxiaojt/win-down/blob/main/README.md) | **简体中文**

Windows 平台多线程下载器，核心引擎使用 Rust + WinHTTP 实现，通过 Node-API (N-API) 暴露给 Node.js，提供完整的 TypeScript 类型支持。

> 仅支持 Windows（依赖 WinHTTP 与 Win32 API）。

## 特性

- **多线程分片下载**：自动探测 `Range` 支持，按分片并发下载
- **自适应降级**：服务器不支持分片等场景下自动降级为单连接，并告知原因
- **断点续传**：基于分片进度恢复下载，校验器变化时自动重下
- **镜像源容错**：配置多个镜像 URL，失败时自动切换
- **重试与退避**：可配置的指数退避重试
- **完整性校验**：SHA-256 / MD5 / CRC32
- **并发调度**：优先级队列 + 按系统资源自适应调节并发
- **资源策略**：`balanced` / `favor-other-apps` / `max-performance`
- **代理支持**：默认使用系统代理，也可显式指定
- **事件驱动**：进度、状态、降级、统计、日志全部通过 EventEmitter 推送

## 安装

需要 Node.js >= 18 与 Windows 环境。

```bash
npm install win-down
```

## 快速开始

```ts
import { Downloader } from 'win-down';

// 一次性下载
await Downloader.download('https://example.com/file.zip', 'D:/Downloads/file.zip');

// 或使用实例管理多个任务
const downloader = new Downloader({ maxConcurrency: 8, resume: true });

const task = downloader.addTask({
  url: 'https://example.com/file.zip',
  destination: 'D:/Downloads/file.zip',
  checksum: { algorithm: 'sha256', value: '...' },
});

task.on('progress', (e) => {
  console.log(`${e.percent.toFixed(1)}%  ${(e.speedBps / 1e6).toFixed(1)} MB/s`);
});

await task.wait(); // 完成 resolve，失败 reject(DownloadError)

downloader.shutdown();
```

CJS 同样可用：

```js
const { Downloader } = require('win-down');
```

## 文档

详细 API 说明见 [`docs/cn/`](https://github.com/zhuxiaojt/win-down/blob/main/docs/cn/)：

| 文档 | 内容 |
| --- | --- |
| [Downloader API](https://github.com/zhuxiaojt/win-down/blob/main/docs/cn/downloader.md) | 下载器实例、全部配置项、全局统计、事件、资源策略 |
| [DownloadTask API](https://github.com/zhuxiaojt/win-down/blob/main/docs/cn/task.md) | 任务配置、属性、方法、事件、进度事件字段、断点续传 |
| [错误码与降级原因](https://github.com/zhuxiaojt/win-down/blob/main/docs/cn/errors.md) | DownloadError 错误码、DegradeReason、任务状态机 |

## 架构

```
src/                  # TypeScript 绑定层（CJS 主实现 + ESM 重导出）
native/               # Rust 核心引擎（WinHTTP / Win32 / BCrypt）
  src/engine/         #   调度器、工作线程、传输、续传、监控、生命周期
scripts/build.js      # cargo 构建 + 拷贝 .node
test/                 # 本地 HTTP 服务器 + 集成测试
```

## License

MIT
