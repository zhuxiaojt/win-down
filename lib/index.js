'use strict';

const path = require('node:path');
const { EventEmitter } = require('node:events');

let nativeModule = null;

function loadNative() {
  if (nativeModule) {
    return nativeModule;
  }
  const candidates = [
    path.join(__dirname, '..', 'native', 'win_downloader.node'),
    path.join(__dirname, '..', 'win_downloader.node'),
  ];
  let lastError = null;
  for (const file of candidates) {
    try {
      nativeModule = require(file);
      return nativeModule;
    } catch (err) {
      lastError = err;
    }
  }
  const hint = 'Native module not found. Run "npm run build" first.';
  throw new Error(`${hint}${lastError ? ` (${lastError.message})` : ''}`);
}

class DownloadError extends Error {
  constructor(code, message) {
    super(message || code || 'download failed');
    this.name = 'DownloadError';
    this.code = code || 'ERR_UNKNOWN';
  }
}

const TERMINAL_STATES = new Set(['completed', 'failed', 'cancelled']);

class DownloadTask extends EventEmitter {
  constructor(downloader, id, options) {
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

    this._waiters = [];
    this._lastEvent = null;
    this._completedEvent = null;
  }

  get done() {
    return TERMINAL_STATES.has(this.state);
  }

  wait() {
    if (this.state === 'completed') {
      if (this._completedEvent) {
        return Promise.resolve(this._completedEvent);
      }
      // completed 事件尚未到达，继续等待，避免返回缺字段的 state 事件。
      return new Promise((resolve, reject) => {
        this._waiters.push({ resolve, reject });
      });
    }
    if (this.state === 'failed') {
      return Promise.reject(this.error || new DownloadError('ERR_FAILED', 'download failed'));
    }
    if (this.state === 'cancelled') {
      return Promise.reject(new DownloadError('ERR_CANCELLED', 'download cancelled'));
    }
    return new Promise((resolve, reject) => {
      this._waiters.push({ resolve, reject });
    });
  }

  pause() {
    return this.downloader.pause(this.id);
  }

  resume() {
    return this.downloader.resume(this.id);
  }

  cancel(options = {}) {
    return this.downloader.cancel(this.id, options);
  }

  _apply(event) {
    const e = event;
    if (typeof e.totalBytes === 'number' && e.totalBytes >= 0) {
      this.totalBytes = e.totalBytes;
    }
    if (typeof e.downloadedBytes === 'number') {
      this.downloadedBytes = e.downloadedBytes;
    }
    this.percent = e.percent >= 0 ? e.percent : null;
    this.speedBps = e.speedBps || 0;
    this.etaSeconds = e.etaSeconds >= 0 ? e.etaSeconds : null;
    this.activeConnections = e.activeConnections || 0;
    this.segmentsTotal = e.segmentsTotal || 0;
    this.segmentsCompleted = e.segmentsCompleted || 0;
    this.degraded = !!e.degraded;
    this.degradeReason = e.degradeReason || null;
    this.elapsedMs = e.elapsedMs || 0;
    this.resumedBytes = e.resumedBytes || 0;
    this.sessionBytes = e.sessionBytes || 0;
    if (e.finalUrl) this.finalUrl = e.finalUrl;
    if (e.etag) this.etag = e.etag;
    if (e.lastModified) this.lastModified = e.lastModified;
    if (e.contentType) this.contentType = e.contentType;
    if (typeof e.attempts === 'number' && e.attempts > 0) this.attempts = e.attempts;
    if (this._checksumConfigured && typeof e.checksumVerified === 'boolean') {
      this.checksumVerified = e.checksumVerified;
    }
    if (typeof e.skipped === 'boolean') this.skipped = e.skipped;
    if (e.state) this.state = e.state;
    this._lastEvent = e;
  }

  _fail(error) {
    this.error = error;
    const waiters = this._waiters.splice(0);
    for (const w of waiters) {
      w.reject(error);
    }
  }

  _resolve(result) {
    const waiters = this._waiters.splice(0);
    for (const w of waiters) {
      w.resolve(result);
    }
  }
}

class Downloader extends EventEmitter {
  constructor(options = {}) {
    super();
    const { NativeDownloader } = loadNative();
    this._tasks = new Map();
    this._pending = new Map();
    this._probeWaiters = new Map();
    this._tokenSeq = 0;
    this._closed = false;
    this._native = new NativeDownloader(options, (err, event) => {
      if (err) {
        this.emit('warning', err);
        return;
      }
      this._onEvent(event);
    });
  }

  addTask(options = {}) {
    if (this._closed) {
      throw new DownloadError('ERR_CLOSED', 'downloader is closed');
    }
    const id = this._native.addTask(options);
    const task = new DownloadTask(this, id, options);
    this._tasks.set(id, task);
    this.emit('task', task);
    return task;
  }

  getTask(id) {
    return this._tasks.get(id) || null;
  }

  get tasks() {
    return Array.from(this._tasks.values());
  }

  pause(id) {
    return this._invoke('pause', id);
  }

  resume(id) {
    return this._invoke('resume', id);
  }

  cancel(id, options = {}) {
    return this._invoke('cancel', id, !!options.removePartialFile);
  }

  pauseAll() {
    return this._invoke('pauseAll');
  }

  resumeAll() {
    return this._invoke('resumeAll');
  }

  cancelAll(options = {}) {
    return this._invoke('cancelAll', null, !!options.removePartialFile);
  }

  flush() {
    return this._invoke('flush');
  }

  removeTask(id, options = {}) {
    return this._invoke('removeTask', id, !!options.removePartialFile).then(() => {
      this._tasks.delete(id);
    });
  }

  clearCompleted() {
    return this._invoke('clearCompleted').then(() => {
      for (const [id, task] of this._tasks) {
        if (task.done) {
          this._tasks.delete(id);
        }
      }
    });
  }

  getStats() {
    this._ensureOpen();
    return this._native.getStats();
  }

  setMaxConcurrency(value) {
    this._ensureOpen();
    this._native.setMaxConcurrency(value);
    return this;
  }

  setMaxTasksInFlight(value) {
    this._ensureOpen();
    this._native.setMaxTasksInFlight(value);
    return this;
  }

  setPolicy(policy) {
    this._ensureOpen();
    this._native.setPolicy(policy);
    return this;
  }

  _ensureOpen() {
    if (this._closed || !this._native) {
      throw new DownloadError('ERR_CLOSED', 'downloader is closed');
    }
  }

  shutdown() {
    if (this._closed) {
      return;
    }
    this._closed = true;
    const error = new DownloadError('ERR_SHUTDOWN', 'downloader was shut down');
    for (const waiter of this._pending.values()) {
      waiter.reject(error);
    }
    this._pending.clear();
    for (const waiter of this._probeWaiters.values()) {
      clearTimeout(waiter.timer);
      waiter.reject(error);
    }
    this._probeWaiters.clear();
    for (const task of this._tasks.values()) {
      if (task.done) {
        continue;
      }
      // 用最后一次事件快照补出失败事件，保证 failed/state 监听者也能收到通知。
      const event = {
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
      task.emit('failed', error, event);
      this.emit('failed', task, error, event);
    }
    this._native.shutdown();
    this._native = null;
    this.emit('close');
  }

  _invoke(command, arg, flag = false) {
    if (this._closed) {
      return Promise.reject(new DownloadError('ERR_CLOSED', 'downloader is closed'));
    }
    const token = ++this._tokenSeq;
    return new Promise((resolve, reject) => {
      this._pending.set(token, { resolve, reject });
      this._native.invoke(command, token, arg == null ? '' : String(arg), !!flag);
    });
  }

  _probe(url, timeoutMs = 30000) {
    if (this._closed) {
      return Promise.reject(new DownloadError('ERR_CLOSED', 'downloader is closed'));
    }
    const token = ++this._tokenSeq;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this._probeWaiters.delete(token);
        reject(new DownloadError('ERR_PROBE_TIMEOUT', 'probe timed out'));
      }, timeoutMs);
      this._probeWaiters.set(token, { resolve, reject, timer });
      this._native.invoke('probe', token, url);
    });
  }

  _onEvent(event) {
    // shutdown 之后原生侧可能还有排队的 TSFN 事件，直接丢弃，避免覆盖已置为 failed 的状态。
    if (this._closed) {
      return;
    }
    if (event.kind === 'done') {
      const waiter = this._pending.get(event.token);
      if (waiter) {
        this._pending.delete(event.token);
        if (event.ok) {
          waiter.resolve();
        } else {
          waiter.reject(new DownloadError(event.code, event.message));
        }
      }
      this.emit('done', event);
      return;
    }

    if (event.kind === 'probe') {
      const waiter = this._probeWaiters.get(event.token);
      if (waiter) {
        this._probeWaiters.delete(event.token);
        clearTimeout(waiter.timer);
        waiter.resolve(event);
      }
      this.emit('probe', event);
      return;
    }

    if (event.kind === 'stats') {
      this.emit('stats', event);
      return;
    }

    if (event.kind === 'log') {
      this.emit('log', event);
      return;
    }

    const task = event.taskId ? this._tasks.get(event.taskId) : null;
    if (!task) {
      return;
    }

    switch (event.kind) {
      case 'progress':
        task._apply(event);
        task.emit('progress', event);
        this.emit('progress', task, event);
        break;
      case 'state': {
        task._apply(event);
        task.emit('state', event);
        this.emit('state', task, event);
        if (event.state === 'cancelled') {
          task._fail(new DownloadError('ERR_CANCELLED', 'download cancelled'));
          task.emit('cancelled');
        }
        break;
      }
      case 'degraded':
        task.degraded = true;
        task.degradeReason = event.degradeReason || task.degradeReason;
        task.emit('degraded', event);
        this.emit('degraded', task, event);
        break;
      case 'reset':
        task._apply(event);
        task.emit('reset', event);
        this.emit('reset', task, event);
        break;
      case 'completed':
        task._apply(event);
        task._completedEvent = event;
        task.emit('completed', event);
        this.emit('completed', task, event);
        task._resolve(event);
        break;
      case 'failed': {
        task._apply(event);
        const error = new DownloadError(event.code, event.message);
        task._fail(error);
        task.emit('failed', error, event);
        this.emit('failed', task, error, event);
        break;
      }
      default:
        break;
    }
  }

  static async probe(url, options = {}) {
    const downloader = new Downloader(options.engine || {});
    try {
      return await downloader._probe(url, options.timeoutMs);
    } finally {
      downloader.shutdown();
    }
  }

  static async download(url, destination, options = {}) {
    const downloader = new Downloader(options.engine || {});
    try {
      const task = downloader.addTask({
        url,
        destination,
        ...(options.task || {}),
      });
      return await task.wait();
    } finally {
      downloader.shutdown();
    }
  }
}

module.exports = {
  Downloader,
  DownloadTask,
  DownloadError,
};
