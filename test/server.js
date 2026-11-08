'use strict';

const http = require('node:http');

const ETAG = '"wdl-test-1"';
const LAST_MODIFIED = 'Wed, 01 Jan 2025 00:00:00 GMT';

function makeBody(size) {
  const buf = Buffer.allocUnsafe(size);
  let v = 17;
  for (let i = 0; i < size; i += 1) {
    v = (v * 31 + 7) & 0xff;
    buf[i] = v;
  }
  return buf;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function parseRange(header, size) {
  const match = /^bytes=(\d*)-(\d*)$/.exec(header || '');
  if (!match) {
    return null;
  }
  let start = match[1] === '' ? null : Number(match[1]);
  let end = match[2] === '' ? null : Number(match[2]);
  if (start === null && end === null) {
    return null;
  }
  if (start === null) {
    start = Math.max(0, size - end);
    end = size - 1;
  } else if (end === null || end >= size) {
    end = size - 1;
  }
  if (start >= size || start > end) {
    return { invalid: true };
  }
  return { start, end };
}

async function sendSlow(res, buffer, delayMs, chunkSize) {
  const chunk = Math.max(1024, chunkSize || 32768);
  for (let offset = 0; offset < buffer.length; offset += chunk) {
    if (res.destroyed || res.writableEnded) {
      return;
    }
    res.write(buffer.subarray(offset, Math.min(buffer.length, offset + chunk)));
    if (delayMs > 0) {
      await sleep(delayMs);
    } else {
      await new Promise((resolve) => setImmediate(resolve));
    }
  }
  if (!res.destroyed && !res.writableEnded) {
    res.end();
  }
}

async function startServer() {
  const info = {
    requests: [],
    flakyFailures: 3,
  };

  const server = http.createServer(async (req, res) => {
    const url = new URL(req.url, 'http://127.0.0.1');
    const path = url.pathname;
    const size = Math.max(0, Number(url.searchParams.get('size') || 1024 * 1024));
    const delay = Number(url.searchParams.get('delay') || 0);
    const chunkSize = Number(url.searchParams.get('chunk') || 32768);
    const body = () => makeBody(size);
    const record = { method: req.method, path, range: req.headers.range || null, status: 0 };
    info.requests.push(record);

    const setCommon = () => {
      res.setHeader('Content-Type', 'application/octet-stream');
      res.setHeader('ETag', ETAG);
      res.setHeader('Last-Modified', LAST_MODIFIED);
    };

    if (path === '/small') {
      setCommon();
      const buf = body();
      res.statusCode = 200;
      res.setHeader('Content-Length', buf.length);
      record.status = 200;
      if (req.method === 'HEAD') {
        res.end();
      } else {
        res.end(buf);
      }
      return;
    }

    if (path === '/chunked') {
      res.statusCode = 200;
      res.setHeader('Content-Type', 'application/octet-stream');
      res.setHeader('Transfer-Encoding', 'chunked');
      record.status = 200;
      if (req.method === 'HEAD') {
        res.end();
      } else {
        await sendSlow(res, body(), delay, chunkSize);
      }
      return;
    }

    if (path === '/liar') {
      // 宣称支持 Range，但带 Range 的 GET 请求会返回 200 全量。
      setCommon();
      res.setHeader('Accept-Ranges', 'bytes');
      const buf = body();
      if (req.method === 'HEAD') {
        res.statusCode = 200;
        res.setHeader('Content-Length', buf.length);
        record.status = 200;
        res.end();
        return;
      }
      if (req.headers.range) {
        res.statusCode = 200;
        res.setHeader('Content-Length', buf.length);
        record.status = 200;
        res.end(buf);
        return;
      }
      res.statusCode = 200;
      res.setHeader('Content-Length', buf.length);
      record.status = 200;
      res.end(buf);
      return;
    }

    if (path === '/missing') {
      res.statusCode = 404;
      res.setHeader('Content-Length', 0);
      record.status = 404;
      res.end();
      return;
    }

    if (path === '/wrongrange') {
      // 206 的 Content-Range 起点与请求不一致，用于校验分段偏移检查。
      setCommon();
      res.setHeader('Accept-Ranges', 'bytes');
      const buf = body();
      if (req.method === 'HEAD') {
        res.statusCode = 200;
        res.setHeader('Content-Length', buf.length);
        record.status = 200;
        res.end();
        return;
      }
      if (req.headers.range) {
        const parsed = parseRange(req.headers.range, buf.length);
        if (!parsed || parsed.invalid) {
          res.statusCode = 416;
          res.setHeader('Content-Range', `bytes */${buf.length}`);
          record.status = 416;
          res.end();
          return;
        }
        const len = parsed.end - parsed.start + 1;
        res.statusCode = 206;
        res.setHeader('Content-Range', `bytes 0-${len - 1}/${buf.length}`);
        res.setHeader('Content-Length', len);
        record.status = 206;
        res.end(buf.subarray(0, len));
        return;
      }
      res.statusCode = 200;
      res.setHeader('Content-Length', buf.length);
      record.status = 200;
      res.end(buf);
      return;
    }

    if (path === '/nohead') {
      // 拒绝 HEAD（405 且带 Content-Length: 0），GET 仍支持 Range；用于验证探针兜底路径。
      setCommon();
      res.setHeader('Accept-Ranges', 'bytes');
      const buf = body();
      if (req.method === 'HEAD') {
        res.statusCode = 405;
        res.setHeader('Content-Length', 0);
        record.status = 405;
        res.end();
        return;
      }
      const rangeHeader = req.headers.range;
      if (rangeHeader) {
        const parsed = parseRange(rangeHeader, buf.length);
        if (!parsed || parsed.invalid) {
          res.statusCode = 416;
          res.setHeader('Content-Range', `bytes */${buf.length}`);
          record.status = 416;
          res.end();
          return;
        }
        res.statusCode = 206;
        res.setHeader('Content-Range', `bytes ${parsed.start}-${parsed.end}/${buf.length}`);
        res.setHeader('Content-Length', parsed.end - parsed.start + 1);
        record.status = 206;
        res.end(buf.subarray(parsed.start, parsed.end + 1));
        return;
      }
      res.statusCode = 200;
      res.setHeader('Content-Length', buf.length);
      record.status = 200;
      res.end(buf);
      return;
    }

    if (path === '/norange') {
      setCommon();
      const buf = body();
      res.statusCode = 200;
      res.setHeader('Content-Length', buf.length);
      record.status = 200;
      if (req.method === 'HEAD') {
        res.end();
      } else {
        res.end(buf);
      }
      return;
    }

    if (path === '/flaky') {
      setCommon();
      res.setHeader('Accept-Ranges', 'bytes');
      const buf = body();
      if (req.method !== 'HEAD' && req.headers.range && info.flakyFailures > 0) {
        info.flakyFailures -= 1;
        res.statusCode = 429;
        res.setHeader('Content-Length', 0);
        record.status = 429;
        res.end();
        return;
      }
      const rangeHeader = req.headers.range;
      if (rangeHeader && req.method !== 'HEAD') {
        const parsed = parseRange(rangeHeader, buf.length);
        if (!parsed || parsed.invalid) {
          res.statusCode = 416;
          res.setHeader('Content-Range', `bytes */${buf.length}`);
          record.status = 416;
          res.end();
          return;
        }
        res.statusCode = 206;
        res.setHeader('Content-Range', `bytes ${parsed.start}-${parsed.end}/${buf.length}`);
        res.setHeader('Content-Length', parsed.end - parsed.start + 1);
        record.status = 206;
        await sendSlow(res, buf.subarray(parsed.start, parsed.end + 1), delay, chunkSize);
        return;
      }
      res.statusCode = 200;
      res.setHeader('Content-Length', buf.length);
      record.status = 200;
      if (req.method === 'HEAD') {
        res.end();
      } else {
        await sendSlow(res, buf, delay, chunkSize);
      }
      return;
    }

    // 默认 /file：支持 Range + ETag，可选限速。
    setCommon();
    res.setHeader('Accept-Ranges', 'bytes');
    const buf = body();
    const rangeHeader = req.headers.range;

    if (req.method === 'HEAD') {
      res.statusCode = 200;
      res.setHeader('Content-Length', buf.length);
      record.status = 200;
      res.end();
      return;
    }

    if (rangeHeader) {
      const ifRange = req.headers['if-range'];
      if (ifRange && ifRange !== ETAG) {
        res.statusCode = 200;
        res.setHeader('Content-Length', buf.length);
        record.status = 200;
        await sendSlow(res, buf, delay, chunkSize);
        return;
      }
      const parsed = parseRange(rangeHeader, buf.length);
      if (!parsed || parsed.invalid) {
        res.statusCode = 416;
        res.setHeader('Content-Range', `bytes */${buf.length}`);
        record.status = 416;
        res.end();
        return;
      }
      res.statusCode = 206;
      res.setHeader('Content-Range', `bytes ${parsed.start}-${parsed.end}/${buf.length}`);
      res.setHeader('Content-Length', parsed.end - parsed.start + 1);
      record.status = 206;
      await sendSlow(res, buf.subarray(parsed.start, parsed.end + 1), delay, chunkSize);
      return;
    }

    res.statusCode = 200;
    res.setHeader('Content-Length', buf.length);
    record.status = 200;
    await sendSlow(res, buf, delay, chunkSize);
  });

  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const port = server.address().port;

  return {
    port,
    info,
    baseUrl: `http://127.0.0.1:${port}`,
    close: () =>
      new Promise((resolve, reject) => {
        server.close((err) => (err ? reject(err) : resolve()));
      }),
  };
}

module.exports = { startServer, makeBody, ETAG };
