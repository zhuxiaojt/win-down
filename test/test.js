'use strict';

const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const assert = require('node:assert');
const { startServer, makeBody, ETAG } = require('./server');
const { Downloader, DownloadError } = require('../lib/index.js');

const TMP = path.join(__dirname, 'tmp');
const MiB = 1024 * 1024;

function cleanTmp() {
  fs.rmSync(TMP, { recursive: true, force: true });
  fs.mkdirSync(TMP, { recursive: true });
}

function dest(name) {
  return path.join(TMP, name);
}

function sha256(buf) {
  return crypto.createHash('sha256').update(buf).digest('hex');
}

function assertSame(actual, expected, label) {
  assert.strictEqual(actual.length, expected.length, `${label}: size mismatch`);
  for (let i = 0; i < expected.length; i += 1) {
    if (actual[i] !== expected[i]) {
      assert.fail(`${label}: byte mismatch at ${i} (${actual[i]} != ${expected[i]})`);
    }
  }
}

function filesInTmp() {
  return fs.readdirSync(TMP).sort();
}

function withTimeout(promise, ms, label) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${label}: timed out after ${ms}ms`)), ms);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

function waitFor(predicate, ms, label) {
  return new Promise((resolve, reject) => {
    const started = Date.now();
    const tick = () => {
      let value;
      try {
        value = predicate();
      } catch (err) {
        return reject(err);
      }
      if (value) {
        return resolve(value);
      }
      if (Date.now() - started > ms) {
        return reject(new Error(`${label}: condition not met within ${ms}ms`));
      }
      setTimeout(tick, 10);
    };
    tick();
  });
}

const results = [];

async function test(name, fn) {
  const started = Date.now();
  try {
    cleanTmp();
    await withTimeout(fn(), 60000, name);
    const elapsed = Date.now() - started;
    results.push({ name, ok: true, elapsed });
    console.log(`PASS  ${name} (${elapsed}ms)`);
  } catch (err) {
    const elapsed = Date.now() - started;
    results.push({ name, ok: false, elapsed, error: err });
    console.log(`FAIL  ${name} (${elapsed}ms)\n      ${err && err.stack ? err.stack.split('\n').join('\n      ') : err}`);
  }
}

async function main() {
  const server = await startServer();
  const base = server.baseUrl;

  await test('small file uses single connection', async () => {
    const size = 400 * 1024;
    const body = makeBody(size);
    const dl = new Downloader({ smallFileThreshold: 2 * MiB, segmentSize: 64 * 1024 });
    try {
      const task = dl.addTask({ url: `${base}/small?size=${size}`, destination: dest('small.bin') });
      const event = await withTimeout(task.wait(), 20000, 'small file');
      assert.strictEqual(event.state, 'completed');
      assert.strictEqual(event.connectionsUsed, 1);
      assert.strictEqual(task.totalBytes, size);
      assertSame(fs.readFileSync(dest('small.bin')), body, 'small file content');
      assert.deepStrictEqual(filesInTmp(), ['small.bin']);
    } finally {
      dl.shutdown();
    }
  });

  await test('segmented download with sha256 checksum', async () => {
    const size = 6 * MiB;
    const body = makeBody(size);
    const dl = new Downloader({
      maxConcurrency: 8,
      segmentSize: MiB,
      smallFileThreshold: 1024,
      maxSegmentsPerTask: 16,
    });
    try {
      const task = dl.addTask({
        url: `${base}/file?size=${size}`,
        destination: dest('seg.bin'),
        checksum: { algorithm: 'sha256', value: sha256(body) },
      });
      const maxActive = { value: 0 };
      dl.on('progress', (_t, event) => {
        if (event.activeConnections > maxActive.value) {
          maxActive.value = event.activeConnections;
        }
      });
      const event = await withTimeout(task.wait(), 30000, 'segmented download');
      assert.strictEqual(event.state, 'completed');
      assert.strictEqual(task.checksumVerified, true);
      assert.strictEqual(event.checksumVerified, true);
      assert.strictEqual(task.totalBytes, size);
      assertSame(fs.readFileSync(dest('seg.bin')), body, 'segmented content');
      assert.ok(maxActive.value > 1, `expected concurrent connections > 1, got ${maxActive.value}`);
      assert.deepStrictEqual(filesInTmp(), ['seg.bin']);
    } finally {
      dl.shutdown();
    }
  });

  await test('range-ignored server degrades to single connection', async () => {
    const size = 2 * MiB;
    const body = makeBody(size);
    const dl = new Downloader({
      maxConcurrency: 8,
      segmentSize: 256 * 1024,
      smallFileThreshold: 1024,
    });
    const reasons = [];
    try {
      dl.on('degraded', (_t, event) => reasons.push(event.degradeReason));
      const task = dl.addTask({ url: `${base}/liar?size=${size}`, destination: dest('liar.bin') });
      const event = await withTimeout(task.wait(), 30000, 'liar');
      assert.strictEqual(event.state, 'completed');
      assert.strictEqual(task.degraded, true);
      assert.ok(reasons.includes('range-ignored'), `degrade reasons: ${JSON.stringify(reasons)}`);
      assertSame(fs.readFileSync(dest('liar.bin')), body, 'liar content');
      assert.deepStrictEqual(filesInTmp(), ['liar.bin']);
    } finally {
      dl.shutdown();
    }
  });

  await test('unknown length (chunked) reports null percent', async () => {
    const size = 800 * 1024;
    const body = makeBody(size);
    const dl = new Downloader({ maxConcurrency: 4 });
    const percents = [];
    try {
      const task = dl.addTask({
        url: `${base}/chunked?size=${size}&delay=20&chunk=32768`,
        destination: dest('chunked.bin'),
      });
      task.on('progress', (event) => percents.push(event.percent));
      const event = await withTimeout(task.wait(), 30000, 'chunked');
      assert.strictEqual(event.state, 'completed');
      assert.strictEqual(task.totalBytes, size);
      assertSame(fs.readFileSync(dest('chunked.bin')), body, 'chunked content');
      assert.ok(percents.length > 0, 'expected progress events');
      assert.ok(percents.some((p) => p === -1), 'expected unknown percent before total known');
    } finally {
      dl.shutdown();
    }
  });

  await test('multiple tasks share the scheduler', async () => {
    const sizes = [1 * MiB, 2 * MiB, 3 * MiB, 4 * MiB];
    const bodies = sizes.map((s) => makeBody(s));
    const dl = new Downloader({
      maxConcurrency: 4,
      maxTasksInFlight: 2,
      segmentSize: 256 * 1024,
      smallFileThreshold: 1024,
    });
    try {
      const tasks = sizes.map((size, i) =>
        dl.addTask({ url: `${base}/file?size=${size}`, destination: dest(`multi-${i}.bin`) }),
      );
      await withTimeout(Promise.all(tasks.map((t) => t.wait())), 60000, 'multi tasks');
      tasks.forEach((t, i) => {
        assert.strictEqual(t.state, 'completed');
        assertSame(fs.readFileSync(dest(`multi-${i}.bin`)), bodies[i], `multi-${i} content`);
      });
      const stats = dl.getStats();
      assert.strictEqual(stats.activeTasks, 0);
      assert.deepStrictEqual(
        filesInTmp(),
        ['multi-0.bin', 'multi-1.bin', 'multi-2.bin', 'multi-3.bin'],
      );
    } finally {
      dl.shutdown();
    }
  });

  await test('pause saves sidecar and resume completes', async () => {
    const size = 4 * MiB;
    const body = makeBody(size);
    const dl = new Downloader({
      maxConcurrency: 4,
      segmentSize: 256 * 1024,
      smallFileThreshold: 1024,
      resume: true,
    });
    try {
      const target = dest('resume.bin');
      const task = dl.addTask({ url: `${base}/file?size=${size}&delay=12&chunk=65536`, destination: target });
      await waitFor(() => task.downloadedBytes > 256 * 1024, 20000, 'progress before pause');
      await withTimeout(task.pause(), 10000, 'pause command');
      assert.strictEqual(task.state, 'paused');
      await waitFor(() => fs.existsSync(`${target}.wdl`), 5000, 'sidecar after pause');

      await withTimeout(task.resume(), 10000, 'resume command');
      const event = await withTimeout(task.wait(), 60000, 'resume complete');
      assert.strictEqual(event.state, 'completed');
      assertSame(fs.readFileSync(target), body, 'resume content');
      assert.strictEqual(fs.existsSync(`${target}.wdl`), false, 'sidecar should be removed');
      assert.deepStrictEqual(filesInTmp(), ['resume.bin']);
    } finally {
      dl.shutdown();
    }
  });

  await test('static probe returns metadata', async () => {
    const size = 1234567;
    const result = await withTimeout(Downloader.probe(`${base}/file?size=${size}`), 20000, 'probe');
    assert.strictEqual(result.kind, 'probe');
    assert.strictEqual(result.ok, true);
    assert.strictEqual(result.contentLength, size);
    assert.strictEqual(result.acceptRanges, true);
    assert.strictEqual(result.etag, ETAG);
  });

  await test('retries transient 429 failures', async () => {
    server.info.flakyFailures = 3;
    const size = 512 * 1024;
    const body = makeBody(size);
    const dl = new Downloader({
      maxConcurrency: 4,
      segmentSize: 128 * 1024,
      smallFileThreshold: 1024,
      maxRetries: 6,
      baseRetryMs: 50,
    });
    try {
      const task = dl.addTask({ url: `${base}/flaky?size=${size}`, destination: dest('flaky.bin') });
      const event = await withTimeout(task.wait(), 40000, 'flaky');
      assert.strictEqual(event.state, 'completed');
      assertSame(fs.readFileSync(dest('flaky.bin')), body, 'flaky content');
      const failures = server.info.requests.filter(
        (r) => r.path === '/flaky' && r.method === 'GET' && r.status === 429,
      );
      assert.ok(failures.length > 0, 'expected 429 attempts to be recorded');
    } finally {
      dl.shutdown();
    }
  });

  await test('missing destination with overwrite=error fails with code', async () => {
    const target = dest('exists.bin');
    fs.writeFileSync(target, 'keep');
    const dl = new Downloader({});
    try {
      const task = dl.addTask({ url: `${base}/small?size=1024`, destination: target, overwrite: 'error' });
      await assert.rejects(withTimeout(task.wait(), 20000, 'overwrite'), (err) => {
        assert.ok(err instanceof DownloadError);
        assert.strictEqual(err.code, 'ERR_FILE_EXISTS');
        return true;
      });
      assert.strictEqual(fs.readFileSync(target, 'utf8'), 'keep');
    } finally {
      dl.shutdown();
    }
  });

  await test('cancel during download stops and removes partial metadata', async () => {
    const dl = new Downloader({
      maxConcurrency: 4,
      segmentSize: 256 * 1024,
      smallFileThreshold: 1024,
      resume: true,
    });
    try {
      const target = dest('cancel.bin');
      const task = dl.addTask({ url: `${base}/file?size=${4 * MiB}&delay=12&chunk=65536`, destination: target });
      await waitFor(() => task.downloadedBytes > 128 * 1024, 20000, 'progress before cancel');
      await withTimeout(task.cancel(), 10000, 'cancel command');
      assert.strictEqual(task.state, 'cancelled');
      await waitFor(() => !fs.existsSync(`${target}.wdl`), 5000, 'sidecar removed after cancel');
    } finally {
      dl.shutdown();
    }
  });

  await test('0-byte file is created', async () => {
    const dl = new Downloader({});
    try {
      const task = dl.addTask({ url: `${base}/file?size=0`, destination: dest('empty.bin') });
      const event = await withTimeout(task.wait(), 20000, 'zero byte');
      assert.strictEqual(event.state, 'completed');
      assert.ok(fs.existsSync(dest('empty.bin')), 'destination should exist');
      assert.strictEqual(fs.statSync(dest('empty.bin')).size, 0);
    } finally {
      dl.shutdown();
    }
  });

  await test('overwrite skip keeps existing file', async () => {
    fs.writeFileSync(dest('skip.bin'), 'keep');
    const dl = new Downloader({});
    try {
      const task = dl.addTask({
        url: `${base}/small?size=1024`,
        destination: dest('skip.bin'),
        overwrite: 'skip',
      });
      const event = await withTimeout(task.wait(), 20000, 'skip');
      assert.strictEqual(event.state, 'completed');
      assert.strictEqual(event.skipped, true);
      assert.strictEqual(fs.readFileSync(dest('skip.bin'), 'utf8'), 'keep');
    } finally {
      dl.shutdown();
    }
  });

  await test('overwrite rename writes to a new file', async () => {
    fs.writeFileSync(dest('ren.bin'), 'keep');
    const dl = new Downloader({});
    try {
      const task = dl.addTask({
        url: `${base}/small?size=1024`,
        destination: dest('ren.bin'),
        overwrite: 'rename',
      });
      await withTimeout(task.wait(), 20000, 'rename');
      assert.strictEqual(fs.readFileSync(dest('ren.bin'), 'utf8'), 'keep');
      const files = filesInTmp();
      assert.strictEqual(files.length, 2, `files: ${JSON.stringify(files)}`);
      const written = files.find((f) => f !== 'ren.bin');
      assert.ok(written, 'expected a renamed destination');
      assertSame(fs.readFileSync(dest(written)), makeBody(1024), 'renamed content');
    } finally {
      dl.shutdown();
    }
  });

  await test('probe reports HTTP errors', async () => {
    const result = await withTimeout(Downloader.probe(`${base}/missing`), 20000, 'probe 404');
    assert.strictEqual(result.statusCode, 404);
    assert.strictEqual(result.ok, false);
    assert.ok(result.message.includes('404'));
  });

  await test('mismatched Content-Range degrades to single connection', async () => {
    const size = 2 * MiB;
    const body = makeBody(size);
    const dl = new Downloader({
      maxConcurrency: 8,
      segmentSize: 256 * 1024,
      smallFileThreshold: 1024,
    });
    const reasons = [];
    try {
      dl.on('degraded', (_t, event) => reasons.push(event.degradeReason));
      const task = dl.addTask({ url: `${base}/wrongrange?size=${size}`, destination: dest('wrong.bin') });
      const event = await withTimeout(task.wait(), 30000, 'wrongrange');
      assert.strictEqual(event.state, 'completed');
      assert.strictEqual(task.degraded, true);
      assert.ok(reasons.includes('range-mismatch'), `reasons: ${JSON.stringify(reasons)}`);
      assertSame(fs.readFileSync(dest('wrong.bin')), body, 'wrongrange content');
    } finally {
      dl.shutdown();
    }
  });

  await test('removeTask and clearCompleted release tasks', async () => {
    const dl = new Downloader({});
    try {
      const first = dl.addTask({ url: `${base}/small?size=1024`, destination: dest('r1.bin') });
      await withTimeout(first.wait(), 20000, 'remove first');
      const second = dl.addTask({ url: `${base}/small?size=2048`, destination: dest('r2.bin') });
      await withTimeout(second.wait(), 20000, 'remove second');
      assert.strictEqual(dl.tasks.length, 2);
      await withTimeout(dl.removeTask(first.id), 10000, 'removeTask');
      assert.strictEqual(dl.tasks.length, 1);
      await withTimeout(dl.clearCompleted(), 10000, 'clearCompleted');
      assert.strictEqual(dl.tasks.length, 0);
    } finally {
      dl.shutdown();
    }
  });

  await test('pause after completion reports terminal state', async () => {
    const dl = new Downloader({});
    try {
      const task = dl.addTask({ url: `${base}/small?size=1024`, destination: dest('p.bin') });
      await withTimeout(task.wait(), 20000, 'terminal pause');
      await assert.rejects(withTimeout(task.pause(), 10000, 'terminal pause call'), (err) => {
        assert.strictEqual(err.code, 'ERR_TASK_TERMINAL');
        return true;
      });
    } finally {
      dl.shutdown();
    }
  });

  await test('shutdown rejects pending waiters', async () => {
    const dl = new Downloader({});
    const task = dl.addTask({
      url: `${base}/file?size=${4 * MiB}&delay=12&chunk=65536`,
      destination: dest('shutdown.bin'),
    });
    let taskFailed = null;
    let dlFailed = null;
    task.on('failed', (err, event) => {
      taskFailed = { code: err.code, state: event && event.state };
    });
    dl.on('failed', (_task, err, event) => {
      dlFailed = { code: err.code, state: event && event.state };
    });
    const pending = task.wait().then(
      () => ({ ok: true }),
      (err) => ({ ok: false, code: err.code }),
    );
    await waitFor(() => task.downloadedBytes > 128 * 1024, 20000, 'progress before shutdown');
    dl.shutdown();
    const result = await withTimeout(pending, 10000, 'shutdown pending');
    assert.strictEqual(result.ok, false);
    assert.strictEqual(result.code, 'ERR_SHUTDOWN');
    assert.ok(taskFailed, 'task failed listener was notified');
    assert.strictEqual(taskFailed.code, 'ERR_SHUTDOWN');
    assert.strictEqual(taskFailed.state, 'failed');
    assert.ok(dlFailed, 'downloader failed listener was notified');
    assert.strictEqual(dlFailed.code, 'ERR_SHUTDOWN');
  });

  await test('removeTask accepts paused task', async () => {
    const dl = new Downloader({ resume: true });
    try {
      const task = dl.addTask({
        url: `${base}/file?size=${8 * MiB}&delay=8&chunk=65536`,
        destination: dest('remove-paused.bin'),
      });
      await waitFor(() => task.downloadedBytes > 128 * 1024, 20000, 'progress before pause');
      await withTimeout(task.pause(), 10000, 'pause before remove');
      assert.strictEqual(task.state, 'paused');
      assert.ok(fs.existsSync(`${dest('remove-paused.bin')}.wdl`), 'sidecar exists after pause');
      await withTimeout(dl.removeTask(task.id), 10000, 'remove paused task');
      assert.strictEqual(dl.tasks.length, 0);
      assert.ok(!fs.existsSync(`${dest('remove-paused.bin')}.wdl`), 'sidecar removed with task');
    } finally {
      dl.shutdown();
    }
  });

  await test('probe falls back when HEAD is rejected', async () => {
    const size = 4 * MiB;
    const body = makeBody(size);
    const probed = await withTimeout(
      Downloader.probe(`${base}/nohead?size=${size}`, { timeoutMs: 20000 }),
      20000,
      'probe nohead',
    );
    assert.strictEqual(probed.ok, true);
    assert.strictEqual(probed.contentLength, size);
    assert.strictEqual(probed.acceptRanges, true);

    const dl = new Downloader({ smallFileThreshold: 1024, segmentSize: 512 * 1024, maxConcurrency: 8 });
    try {
      const task = dl.addTask({ url: `${base}/nohead?size=${size}`, destination: dest('nohead.bin') });
      const event = await withTimeout(task.wait(), 30000, 'nohead download');
      assert.strictEqual(event.state, 'completed');
      assert.strictEqual(task.totalBytes, size);
      assertSame(fs.readFileSync(dest('nohead.bin')), body, 'nohead content');
    } finally {
      dl.shutdown();
    }
  });

  await test('cancel keeps partial file by default', async () => {
    const dl = new Downloader({});
    try {
      const task = dl.addTask({
        url: `${base}/file?size=${8 * MiB}&delay=8&chunk=65536`,
        destination: dest('cancel-keep.bin'),
      });
      await waitFor(() => task.downloadedBytes > 128 * 1024, 20000, 'progress before cancel');
      await withTimeout(dl.cancel(task.id), 10000, 'cancel keep');
      assert.strictEqual(task.state, 'cancelled');
      assert.ok(fs.existsSync(dest('cancel-keep.bin')), 'partial file kept');
    } finally {
      dl.shutdown();
    }
  });

  await test('cancel can remove partial file', async () => {
    const dl = new Downloader({});
    try {
      const task = dl.addTask({
        url: `${base}/file?size=${8 * MiB}&delay=8&chunk=65536`,
        destination: dest('cancel-remove.bin'),
      });
      await waitFor(() => task.downloadedBytes > 128 * 1024, 20000, 'progress before cancel');
      await withTimeout(dl.cancel(task.id, { removePartialFile: true }), 10000, 'cancel remove');
      assert.strictEqual(task.state, 'cancelled');
      assert.ok(!fs.existsSync(dest('cancel-remove.bin')), 'partial file removed');
    } finally {
      dl.shutdown();
    }
  });

  await test('removeTask can delete partial file', async () => {
    const dl = new Downloader({ resume: true });
    try {
      const task = dl.addTask({
        url: `${base}/file?size=${8 * MiB}&delay=8&chunk=65536`,
        destination: dest('remove-file.bin'),
      });
      await waitFor(() => task.downloadedBytes > 128 * 1024, 20000, 'progress before pause');
      await withTimeout(task.pause(), 10000, 'pause before remove');
      await withTimeout(
        dl.removeTask(task.id, { removePartialFile: true }),
        10000,
        'remove task with file',
      );
      assert.strictEqual(dl.tasks.length, 0);
      assert.ok(!fs.existsSync(dest('remove-file.bin')), 'partial file removed');
      assert.ok(!fs.existsSync(`${dest('remove-file.bin')}.wdl`), 'sidecar removed');
    } finally {
      dl.shutdown();
    }
  });

  await test('raising maxConcurrency takes effect immediately', async () => {
    const dl = new Downloader({ maxConcurrency: 4 });
    try {
      assert.ok(dl.getStats().effectiveConcurrency <= 4);
      dl.setMaxConcurrency(16);
      assert.strictEqual(dl.getStats().effectiveConcurrency, 16);
    } finally {
      dl.shutdown();
    }
  });

  await test('rejects header injection', async () => {
    assert.throws(() => new Downloader({ headers: { 'X-Test': 'a\r\nX-Evil: 1' } }));
    const dl = new Downloader({});
    try {
      assert.throws(() =>
        dl.addTask({
          url: `${base}/small?size=1024`,
          destination: dest('hdr.bin'),
          headers: { 'X-Test': 'a\nb' },
        }),
      );
    } finally {
      dl.shutdown();
    }
  });

  await server.close();

  const failed = results.filter((r) => !r.ok).length;
  console.log('');
  if (failed > 0) {
    console.log(`${failed}/${results.length} test(s) failed`);
    process.exitCode = 1;
  } else {
    console.log(`all ${results.length} tests passed`);
  }
}

main().catch((err) => {
  console.error(err);
  process.exitCode = 1;
});
