import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const _mod = require('../cjs/index.cjs') as {
  Downloader: typeof import('../cjs/index.cjs').Downloader;
  DownloadTask: typeof import('../cjs/index.cjs').DownloadTask;
  DownloadError: typeof import('../cjs/index.cjs').DownloadError;
  default?: {
    Downloader: typeof import('../cjs/index.cjs').Downloader;
    DownloadTask: typeof import('../cjs/index.cjs').DownloadTask;
    DownloadError: typeof import('../cjs/index.cjs').DownloadError;
  };
};

export const Downloader = _mod.Downloader;
export const DownloadTask = _mod.DownloadTask;
export const DownloadError = _mod.DownloadError;
export default _mod.default ?? _mod;

export type * from '../cjs/index.cjs';
