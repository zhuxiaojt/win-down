/// <reference types="node" />

import { EventEmitter } from 'node:events';

export type ResourcePolicy = 'balanced' | 'favor-other-apps' | 'max-performance';

export type OverwriteMode = 'error' | 'overwrite' | 'rename' | 'skip';

export type TaskState =
  | 'queued'
  | 'probing'
  | 'downloading'
  | 'retrying'
  | 'paused'
  | 'verifying'
  | 'completed'
  | 'failed'
  | 'cancelled';

export type DegradeReason =
  | 'none'
  | 'no-range-support'
  | 'unknown-length'
  | 'small-file'
  | 'range-ignored'
  | 'range-mismatch'
  | 'content-encoding'
  | 'validator-changed'
  | 'remote-volume'
  | 'server-error-storm';

export type ChecksumAlgorithm = 'sha256' | 'md5' | 'crc32';

export interface DownloaderOptions {
  maxConcurrency?: number;
  maxTasksInFlight?: number;
  policy?: ResourcePolicy;
  segmentSize?: number;
  maxSegmentsPerTask?: number;
  smallFileThreshold?: number;
  maxPerHost?: number;
  connectTimeoutMs?: number;
  idleTimeoutMs?: number;
  maxRetries?: number;
  baseRetryMs?: number;
  maxRetryMs?: number;
  followRedirects?: boolean;
  maxRedirects?: number;
  verifyTls?: boolean;
  proxy?: string;
  userAgent?: string;
  headers?: Record<string, string>;
  /** 断点续传；仅对分段模式生效，小文件/无 Range/chunked 的单流任务暂停后会从头下载。 */
  resume?: boolean;
  preallocate?: boolean;
  progressIntervalMs?: number;
  ioBufferSize?: number;
  createDirectories?: boolean;
}

export interface ChecksumOptions {
  algorithm: ChecksumAlgorithm;
  value: string;
}

export interface DownloadTaskOptions {
  url: string;
  destination: string;
  /** 备用镜像；仅在探测阶段故障切换，下载过程中不会切换镜像。 */
  mirrors?: string[];
  headers?: Record<string, string>;
  expectedSize?: number;
  checksum?: ChecksumOptions;
  priority?: number;
  maxConcurrency?: number;
  overwrite?: OverwriteMode;
  resume?: boolean;
  segmentSize?: number;
}

export interface RemoveFileOptions {
  /**
   * 是否连同目标文件一起删除（默认 false）。
   * 取消时目标文件可能是不完整的；移除已完成任务时删除的是最终文件。
   */
  removePartialFile?: boolean;
}

export interface ProgressEvent {
  kind: 'progress' | 'state' | 'reset' | 'completed' | 'failed';
  taskId: string;
  url: string;
  destination: string;
  state: TaskState;
  totalBytes: number;
  downloadedBytes: number;
  sessionBytes: number;
  resumedBytes: number;
  percent: number;
  speedBps: number;
  averageSpeedBps: number;
  etaSeconds: number;
  activeConnections: number;
  segmentsTotal: number;
  segmentsCompleted: number;
  degraded: boolean;
  degradeReason: DegradeReason;
  elapsedMs: number;
  attempts: number;
  connectionsUsed: number;
  checksumVerified: boolean;
  skipped: boolean;
  finalUrl: string;
  etag: string;
  lastModified: string;
  contentType: string;
  code: string;
  message: string;
}

export interface DegradedEvent {
  kind: 'degraded';
  taskId: string;
  url: string;
  destination: string;
  state: TaskState;
  degraded: true;
  degradeReason: DegradeReason;
  message: string;
}

export interface Stats {
  activeTasks: number;
  queuedTasks: number;
  effectiveConcurrency: number;
  maxConcurrency: number;
  policy: ResourcePolicy;
  cpuUsagePct: number;
  memAvailablePct: number;
  diskWriteLatencyMs: number;
  totalSpeedBps: number;
  totalDownloaded: number;
  totalRemaining: number;
  degradedTasks: number;
}

export interface StatsEvent extends Stats {
  kind: 'stats';
}

export interface LogEvent {
  kind: 'log';
  level: string;
  message: string;
  scope: string;
  taskId?: string;
  url?: string;
}

export interface ProbeResult {
  kind: 'probe';
  ok: boolean;
  statusCode: number;
  contentLength: number;
  acceptRanges: boolean;
  finalUrl: string;
  etag: string;
  lastModified: string;
  contentType: string;
  message: string;
}

export class DownloadError extends Error {
  code: string;
  constructor(code: string, message?: string);
}

export class DownloadTask extends EventEmitter {
  readonly downloader: Downloader;
  readonly id: string;
  readonly url: string;
  readonly destination: string;

  state: TaskState;
  totalBytes: number | null;
  downloadedBytes: number;
  percent: number | null;
  speedBps: number;
  etaSeconds: number | null;
  activeConnections: number;
  segmentsTotal: number;
  segmentsCompleted: number;
  degraded: boolean;
  degradeReason: DegradeReason | null;
  error: DownloadError | null;
  finalUrl: string | null;
  etag: string | null;
  lastModified: string | null;
  contentType: string | null;
  checksumVerified: boolean | null;
  skipped: boolean;
  attempts: number;
  elapsedMs: number;
  resumedBytes: number;
  sessionBytes: number;

  readonly done: boolean;

  wait(): Promise<ProgressEvent>;
  pause(): Promise<void>;
  resume(): Promise<void>;
  cancel(options?: RemoveFileOptions): Promise<void>;

  on(event: 'progress', listener: (event: ProgressEvent) => void): this;
  on(event: 'state', listener: (event: ProgressEvent) => void): this;
  on(event: 'degraded', listener: (event: DegradedEvent) => void): this;
  on(event: 'reset', listener: (event: ProgressEvent) => void): this;
  on(event: 'completed', listener: (event: ProgressEvent) => void): this;
  on(event: 'failed', listener: (error: DownloadError, event: ProgressEvent) => void): this;
  on(event: 'cancelled', listener: () => void): this;
  on(event: string, listener: (...args: any[]) => void): this;
}

export class Downloader extends EventEmitter {
  constructor(options?: DownloaderOptions);

  readonly tasks: DownloadTask[];

  addTask(options: DownloadTaskOptions): DownloadTask;
  getTask(id: string): DownloadTask | null;

  pause(id: string): Promise<void>;
  resume(id: string): Promise<void>;
  /** 取消任务；默认保留不完整的目标文件，传 removePartialFile 可一并删除。 */
  cancel(id: string, options?: RemoveFileOptions): Promise<void>;
  pauseAll(): Promise<void>;
  resumeAll(): Promise<void>;
  /** 取消所有未完成任务；默认保留不完整的目标文件。 */
  cancelAll(options?: RemoveFileOptions): Promise<void>;
  flush(): Promise<void>;
  /** 从任务列表移除（仅终态或已暂停任务）；默认保留目标文件。 */
  removeTask(id: string, options?: RemoveFileOptions): Promise<void>;
  clearCompleted(): Promise<void>;

  getStats(): Stats;
  setMaxConcurrency(value: number): this;
  setMaxTasksInFlight(value: number): this;
  setPolicy(policy: ResourcePolicy): this;

  shutdown(): void;

  on(event: 'task', listener: (task: DownloadTask) => void): this;
  on(event: 'progress', listener: (task: DownloadTask, event: ProgressEvent) => void): this;
  on(event: 'state', listener: (task: DownloadTask, event: ProgressEvent) => void): this;
  on(event: 'degraded', listener: (task: DownloadTask, event: DegradedEvent) => void): this;
  on(event: 'reset', listener: (task: DownloadTask, event: ProgressEvent) => void): this;
  on(event: 'completed', listener: (task: DownloadTask, event: ProgressEvent) => void): this;
  on(event: 'failed', listener: (task: DownloadTask, error: DownloadError, event: ProgressEvent) => void): this;
  on(event: 'stats', listener: (stats: StatsEvent) => void): this;
  on(event: 'log', listener: (event: LogEvent) => void): this;
  on(event: 'warning', listener: (error: Error) => void): this;
  on(event: 'close', listener: () => void): this;
  on(event: string, listener: (...args: any[]) => void): this;

  static probe(url: string, options?: { engine?: DownloaderOptions; timeoutMs?: number }): Promise<ProbeResult>;
  static download(
    url: string,
    destination: string,
    options?: { engine?: DownloaderOptions; task?: Partial<DownloadTaskOptions> },
  ): Promise<ProgressEvent>;
}
