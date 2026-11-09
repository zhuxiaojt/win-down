'use strict';

import path from 'node:path';
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

interface NativeEvent {
  kind: string;
  taskId?: string;
  url?: string;
  destination?: string;
  state?: string;
  code?: string;
  message?: string;
  level?: string;
  scope?: string;
  totalBytes?: number;
  downloadedBytes?: number;
  sessionBytes?: number;
  resumedBytes?: number;
  percent?: number;
  speedBps?: number;
  averageSpeedBps?: number;
  etaSeconds?: number;
  activeConnections?: number;
  segmentsTotal?: number;
  segmentsCompleted?: number;
  degraded?: boolean;
  degradeReason?: string;
  elapsedMs?: number;
  token?: number;
  ok?: boolean;
  statusCode?: number;
  acceptRanges?: boolean;
  contentLength?: number;
  finalUrl?: string;
  etag?: string;
  lastModified?: string;
  contentType?: string;
  contentEncoding?: string;
  attempts?: number;
  connectionsUsed?: number;
  checksumVerified?: boolean;
  skipped?: boolean;
  activeTasks?: number;
  queuedTasks?: number;
  effectiveConcurrency?: number;
  maxConcurrency?: number;
  policy?: string;
  cpuUsagePct?: number;
  memAvailablePct?: number;
  diskWriteLatencyMs?: number;
  totalSpeedBps?: number;
  totalDownloaded?: number;
  totalRemaining?: number;
  degradedTasks?: number;
}

interface NativeStats extends Stats {}

interface NativeDownloader {
  new (options: DownloaderOptions, callback: (err: Error | null, event: NativeEvent) => void): NativeDownloader;
  addTask(options: DownloadTaskOptions): string;
  invoke(command: string, token: number, arg: string, flag: boolean): void;
  getStats(): NativeStats;
  setMaxConcurrency(value: number): void;
  setMaxTasksInFlight(value: number): void;
  setPolicy(policy: string): void;
  shutdown(): void;
}

let nativeModule: { NativeDownloader: NativeDownloader } | null = null;

function loadNative(): { NativeDownloader: NativeDownloader } {
  if (nativeModule) return nativeModule;
  const candidates = [
    path.join(__dirname, '..', 'native', 'win_downloader.node'),
    path.join(__dirname, '..', 'win_downloader.node'),
  ];
  let lastError: Error | null = null;
  for (const file of candidates) {
    try {
      nativeModule = require(file) as { NativeDownloader: NativeDownloader };
      return nativeModule;
    } catch (err) {
      lastError = err as Error;
    }
  }
  const hint = 'Native module not found. Run "npm run build" first.';
  throw new Error(`${hint}${lastError ? ` (${lastError.message})` : ''}`);
}

export class DownloadError extends Error {
  public code: string;
  constructor(code: string, message?: string) {
    super(message || code || 'download failed');
    this.name = 'DownloadError';
    this.code = code || 'ERR_UNKNOWN';
  }
}

const TERMINAL_STATES = new Set<TaskState>(['completed', 'failed', 'cancelled']);

interface Waiter<T = ProgressEvent> {
  resolve: (value: T) => void;
  reject: (reason: unknown) => void;
}

export class DownloadTask extends EventEmitter {
  public readonly downloader: Downloader;
  public readonly id: string;
  public readonly url: string;
  public readonly destination: string;

  public state: TaskState;
  public totalBytes: number | null;
  public downloadedBytes: number;
  public percent: number | null;
  public speedBps: number;
  public etaSeconds: number | null;
  public activeConnections: number;
  public segmentsTotal: number;
  public segmentsCompleted: number;
  public degraded: boolean;
  public degradeReason: DegradeReason | null;
  public error: DownloadError | null;
  public finalUrl: string | null;
  public etag: string | null;
  public lastModified: string | null;
  public contentType: string | null;
  public checksumVerified: boolean | null;
  public skipped: boolean;
  public attempts: number;
  public elapsedMs: number;
  public resumedBytes: number;
  public sessionBytes: number;

  _checksumConfigured: boolean;
  _waiters: Waiter<ProgressEvent>[] = [];
  _lastEvent: NativeEvent | null = null;
  _completedEvent: ProgressEvent | null = null;

  constructor(downloader: Downloader, id: string, options: DownloadTaskOptions) {
    super();
    this.downloader = downloader;
    this.id = id;
    this.url = options.url;
    this.destination = options.destination;
    this.state = 'queued';
    this.totalBytes = null;
    this.downloadedBytes = 0;
    this.percent = null;
    this.speedBps = 0;
    this.etaSeconds = null;
    this.activeConnections = 0;
    this.segmentsTotal = 0;
    this.segmentsCompleted = 0;
    this.degraded = false;
    this.degradeReason = null;
    this.error = null;
    this.finalUrl = null;
    this.etag = null;
    this.lastModified = null;
    this.contentType = null;
    this.checksumVerified = options.checksum ? false : null;
    this._checksumConfigured = !!options.checksum;
    this.skipped = false;
    this.attempts = 0;
    this.elapsedMs = 0;
    this.resumedBytes = 0;
    this.sessionBytes = 0;
  }

  public get done(): boolean {
    return TERMINAL_STATES.has(this.state);
  }

  public wait(): Promise<ProgressEvent> {
    if (this.state === 'completed') {
      if (this._completedEvent) return Promise.resolve(this._completedEvent);
      return new Promise<ProgressEvent>((resolve, reject) => { this._waiters.push({ resolve, reject }); });
    }
    if (this.state === 'failed') return Promise.reject(this.error || new DownloadError('ERR_FAILED', 'download failed'));
    if (this.state === 'cancelled') return Promise.reject(new DownloadError('ERR_CANCELLED', 'download cancelled'));
    return new Promise<ProgressEvent>((resolve, reject) => { this._waiters.push({ resolve, reject }); });
  }

  public pause(): Promise<void> { return this.downloader.pause(this.id); }
  public resume(): Promise<void> { return this.downloader.resume(this.id); }
  public cancel(options: RemoveFileOptions = {}): Promise<void> { return this.downloader.cancel(this.id, options); }

  public _apply(event: NativeEvent): void {
    const e = event;
    if (typeof e.totalBytes === 'number' && e.totalBytes >= 0) this.totalBytes = e.totalBytes;
    if (typeof e.downloadedBytes === 'number') this.downloadedBytes = e.downloadedBytes;
    this.percent = (e.percent ?? -1) >= 0 ? (e.percent as number) : null;
    this.speedBps = e.speedBps || 0;
    this.etaSeconds = (e.etaSeconds ?? -1) >= 0 ? (e.etaSeconds as number) : null;
    this.activeConnections = e.activeConnections || 0;
    this.segmentsTotal = e.segmentsTotal || 0;
    this.segmentsCompleted = e.segmentsCompleted || 0;
    this.degraded = !!e.degraded;
    this.degradeReason = (e.degradeReason as DegradeReason) || null;
    this.elapsedMs = e.elapsedMs || 0;
    this.resumedBytes = e.resumedBytes || 0;
    this.sessionBytes = e.sessionBytes || 0;
    if (e.finalUrl) this.finalUrl = e.finalUrl;
    if (e.etag) this.etag = e.etag;
    if (e.lastModified) this.lastModified = e.lastModified;
    if (e.contentType) this.contentType = e.contentType;
    if (typeof e.attempts === 'number' && e.attempts > 0) this.attempts = e.attempts;
    if (this._checksumConfigured && typeof e.checksumVerified === 'boolean') this.checksumVerified = e.checksumVerified;
    if (typeof e.skipped === 'boolean') this.skipped = e.skipped;
    if (e.state) this.state = e.state as TaskState;
    this._lastEvent = e;
  }

  public _fail(error: DownloadError): void {
    this.error = error;
    const waiters = this._waiters.splice(0);
    for (const w of waiters) w.reject(error);
  }

  public _resolve(result: ProgressEvent): void {
    const waiters = this._waiters.splice(0);
    for (const w of waiters) w.resolve(result);
  }

  public on(event: 'progress', listener: (event: ProgressEvent) => void): this;
  public on(event: 'state', listener: (event: ProgressEvent) => void): this;
  public on(event: 'degraded', listener: (event: DegradedEvent) => void): this;
  public on(event: 'reset', listener: (event: ProgressEvent) => void): this;
  public on(event: 'completed', listener: (event: ProgressEvent) => void): this;
  public on(event: 'failed', listener: (error: DownloadError, event: ProgressEvent) => void): this;
  public on(event: 'cancelled', listener: () => void): this;
  public on(event: string, listener: (...args: any[]) => void): this;
  public on(event: string, listener: (...args: any[]) => void): this { return super.on(event, listener); }
}

interface Pending { resolve: () => void; reject: (err: DownloadError) => void; }

interface ProbeWaiter {
  resolve: (event: ProbeResult) => void;
  reject: (err: DownloadError) => void;
  timer: NodeJS.Timeout;
}

export class Downloader extends EventEmitter {
  private _native: NativeDownloader;
  private _tasks: Map<string, DownloadTask> = new Map();
  private _pending: Map<number, Pending> = new Map();
  private _probeWaiters: Map<number, ProbeWaiter> = new Map();
  private _tokenSeq = 0;
  private _closed = false;

  constructor(options: DownloaderOptions = {}) {
    super();
    const { NativeDownloader } = loadNative();
    this._native = new NativeDownloader(options, (err: Error | null, event: NativeEvent) => {
      if (err) { this.emit('warning', err); return; }
      this._onEvent(event);
    });
  }

  public get tasks(): DownloadTask[] { return Array.from(this._tasks.values()); }

  public addTask(options: DownloadTaskOptions): DownloadTask {
    if (this._closed) throw new DownloadError('ERR_CLOSED', 'downloader is closed');
    const id = this._native.addTask(options);
    const task = new DownloadTask(this, id, options);
    this._tasks.set(id, task);
    this.emit('task', task);
    return task;
  }

  public getTask(id: string): DownloadTask | null { return this._tasks.get(id) || null; }
  public pause(id: string): Promise<void> { return this._invoke('pause', id); }
  public resume(id: string): Promise<void> { return this._invoke('resume', id); }
  public cancel(id: string, options: RemoveFileOptions = {}): Promise<void> { return this._invoke('cancel', id, !!options.removePartialFile); }
  public pauseAll(): Promise<void> { return this._invoke('pauseAll'); }
  public resumeAll(): Promise<void> { return this._invoke('resumeAll'); }
  public cancelAll(options: RemoveFileOptions = {}): Promise<void> { return this._invoke('cancelAll', null, !!options.removePartialFile); }
  public flush(): Promise<void> { return this._invoke('flush'); }

  public removeTask(id: string, options: RemoveFileOptions = {}): Promise<void> {
    return this._invoke('removeTask', id, !!options.removePartialFile).then(() => { this._tasks.delete(id); });
  }

  public clearCompleted(): Promise<void> {
    return this._invoke('clearCompleted').then(() => {
      for (const [id, task] of this._tasks) { if (task.done) this._tasks.delete(id); }
    });
  }

  public getStats(): Stats { this._ensureOpen(); return this._native.getStats() as Stats; }
  public setMaxConcurrency(value: number): this { this._ensureOpen(); this._native.setMaxConcurrency(value); return this; }
  public setMaxTasksInFlight(value: number): this { this._ensureOpen(); this._native.setMaxTasksInFlight(value); return this; }
  public setPolicy(policy: ResourcePolicy): this { this._ensureOpen(); this._native.setPolicy(policy); return this; }

  public shutdown(): void {
    if (this._closed) return;
    this._closed = true;
    const error = new DownloadError('ERR_SHUTDOWN', 'downloader was shut down');
    for (const waiter of this._pending.values()) waiter.reject(error);
    this._pending.clear();
    for (const waiter of this._probeWaiters.values()) { clearTimeout(waiter.timer); waiter.reject(error); }
    this._probeWaiters.clear();
    for (const task of this._tasks.values()) {
      if (task.done) continue;
      const event: NativeEvent = {
        ...(task._lastEvent || {}),
        kind: 'failed',
        taskId: task.id,
        url: task.url,
        destination: task.destination,
        state: 'failed',
        code: 'ERR_SHUTDOWN',
        message: error.message,
      };
      task._apply(event);
      task._fail(error);
      task.emit('failed', error, event as unknown as ProgressEvent);
      this.emit('failed', task, error, event as unknown as ProgressEvent);
    }
    this._native.shutdown();
    // @ts-expect-error
    this._native = null;
    this.emit('close');
  }

  private _ensureOpen(): void {
    if (this._closed || !this._native) throw new DownloadError('ERR_CLOSED', 'downloader is closed');
  }

  private _invoke(command: string, arg?: string | null, flag = false): Promise<void> {
    if (this._closed) return Promise.reject(new DownloadError('ERR_CLOSED', 'downloader is closed'));
    const token = ++this._tokenSeq;
    return new Promise<void>((resolve, reject) => {
      this._pending.set(token, { resolve, reject });
      this._native.invoke(command, token, arg == null ? '' : String(arg), flag);
    });
  }

  private _probe(url: string, timeoutMs = 30000): Promise<ProbeResult> {
    if (this._closed) return Promise.reject(new DownloadError('ERR_CLOSED', 'downloader is closed'));
    const token = ++this._tokenSeq;
    return new Promise<ProbeResult>((resolve, reject) => {
      const timer = setTimeout(() => { this._probeWaiters.delete(token); reject(new DownloadError('ERR_PROBE_TIMEOUT', 'probe timed out')); }, timeoutMs);
      this._probeWaiters.set(token, { resolve, reject, timer });
      this._native.invoke('probe', token, url, false);
    });
  }

  private _onEvent(event: NativeEvent): void {
    if (this._closed) return;
    if (event.kind === 'done') {
      const waiter = this._pending.get(event.token!);
      if (waiter) {
        this._pending.delete(event.token!);
        if (event.ok) waiter.resolve(); else waiter.reject(new DownloadError(event.code || 'ERR_UNKNOWN', event.message || ''));
      }
      this.emit('done', event);
      return;
    }
    if (event.kind === 'probe') {
      const waiter = this._probeWaiters.get(event.token!);
      if (waiter) { this._probeWaiters.delete(event.token!); clearTimeout(waiter.timer); waiter.resolve(event as unknown as ProbeResult); }
      this.emit('probe', event);
      return;
    }
    if (event.kind === 'stats') { this.emit('stats', event as unknown as StatsEvent); return; }
    if (event.kind === 'log') { this.emit('log', event as unknown as LogEvent); return; }
    const task = event.taskId ? this._tasks.get(event.taskId) : null;
    if (!task) return;
    switch (event.kind) {
      case 'progress': task._apply(event); task.emit('progress', event as unknown as ProgressEvent); this.emit('progress', task, event as unknown as ProgressEvent); break;
      case 'state': { task._apply(event); task.emit('state', event as unknown as ProgressEvent); this.emit('state', task, event as unknown as ProgressEvent); if (event.state === 'cancelled') { task._fail(new DownloadError('ERR_CANCELLED', 'download cancelled')); task.emit('cancelled'); } break; }
      case 'degraded': task.degraded = true; task.degradeReason = (event.degradeReason as DegradeReason) || task.degradeReason; task.emit('degraded', event as unknown as DegradedEvent); this.emit('degraded', task, event as unknown as DegradedEvent); break;
      case 'reset': task._apply(event); task.emit('reset', event as unknown as ProgressEvent); this.emit('reset', task, event as unknown as ProgressEvent); break;
      case 'completed': { const pe = event as unknown as ProgressEvent; task._apply(event); task._completedEvent = pe; task.emit('completed', pe); this.emit('completed', task, pe); task._resolve(pe); break; }
      case 'failed': { const pe = event as unknown as ProgressEvent; task._apply(event); const error = new DownloadError(event.code || 'ERR_FAILED', event.message || 'download failed'); task._fail(error); task.emit('failed', error, pe); this.emit('failed', task, error, pe); break; }
    }
  }

  public static async probe(url: string, options: { engine?: DownloaderOptions; timeoutMs?: number } = {}): Promise<ProbeResult> {
    const downloader = new Downloader(options.engine || {});
    try { return await downloader._probe(url, options.timeoutMs); } finally { downloader.shutdown(); }
  }

  public static async download(url: string, destination: string, options: { engine?: DownloaderOptions; task?: Partial<DownloadTaskOptions> } = {}): Promise<ProgressEvent> {
    const downloader = new Downloader(options.engine || {});
    try { return await downloader.addTask({ url, destination, ...(options.task || {}) }).wait(); } finally { downloader.shutdown(); }
  }

  public on(event: 'task', listener: (task: DownloadTask) => void): this;
  public on(event: 'progress', listener: (task: DownloadTask, event: ProgressEvent) => void): this;
  public on(event: 'state', listener: (task: DownloadTask, event: ProgressEvent) => void): this;
  public on(event: 'degraded', listener: (task: DownloadTask, event: DegradedEvent) => void): this;
  public on(event: 'reset', listener: (task: DownloadTask, event: ProgressEvent) => void): this;
  public on(event: 'completed', listener: (task: DownloadTask, event: ProgressEvent) => void): this;
  public on(event: 'failed', listener: (task: DownloadTask, error: DownloadError, event: ProgressEvent) => void): this;
  public on(event: 'stats', listener: (stats: StatsEvent) => void): this;
  public on(event: 'log', listener: (event: LogEvent) => void): this;
  public on(event: 'warning', listener: (error: Error) => void): this;
  public on(event: 'close', listener: () => void): this;
  public on(event: string, listener: (...args: any[]) => void): this;
  public on(event: string, listener: (...args: any[]) => void): this { return super.on(event, listener); }
}

export default { Downloader, DownloadTask, DownloadError };
