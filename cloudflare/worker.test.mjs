import test from 'node:test';
import assert from 'node:assert/strict';
import worker from './worker.mjs';

const originFetch = globalThis.fetch;
const tasks = [];
const ctx = {waitUntil(task) {tasks.push(task);}};
const cache = new Map();
globalThis.caches = {default: {
  async match(key) {return cache.get(key.url)?.clone();},
  async put(key, response) {cache.set(key.url, response);},
}};
const request = (path, options) => new Request(`https://wocli.yhan.fun${path}`, options);
const get = (path, env = {}, options) => worker.fetch(request(path, options), env, ctx);

test('fixed repository, server-side redirects, sanitized metadata, cache, HEAD and BOM-free installer text', async () => {
  const calls = [];
  globalThis.fetch = async (url, options) => {
    calls.push(url);
    assert.equal(options.redirect, 'manual');
    assert.equal(new Headers(options.headers).has('Authorization'), false);
    if (url.includes('api.github.com')) return Response.json({tag_name:'v0.3.4', assets:[{name:'SHA256SUMS', size:10}, {name:'wocli-music-0.3.4-windows-x86_64.zip', size:3}, {name:'evil.exe'}]});
    if (url.startsWith('https://github.com/')) return new Response(null, {status:302, headers:{location:'https://release-assets.githubusercontent.com/example?token=private'}});
    return new Response('abc', {headers:{'content-length':'3', 'content-type':'application/zip', 'set-cookie':'upstream=secret'}});
  };
  try {
    let result = await get('/api/releases/latest?cachebuster=1');
    const metadata = await result.json();
    assert.equal(metadata.assets.length, 2);
    assert.ok(metadata.assets.every(a => a.browser_download_url.startsWith('https://wocli.yhan.fun/')));
    await Promise.all(tasks.splice(0));
    await get('/api/releases/latest?cachebuster=2');
    assert.equal(calls.length, 1);
    result = await get('/download/windows-x86_64');
    assert.equal(result.headers.get('location'), '/releases/download/v0.3.4/wocli-music-0.3.4-windows-x86_64.zip');
    const path = result.headers.get('location');
    result = await get(path, {}, {method:'HEAD', headers:{cookie:'client=secret', Authorization:'Bearer secret'}});
    assert.equal(await result.text(), '');
    assert.equal(result.headers.get('set-cookie'), null);
    assert.equal(result.headers.get('location'), null);
    await Promise.all(tasks.splice(0));
    result = await get(path);
    assert.equal(await result.text(), 'abc');
    assert.equal(calls.length, 3);
    const bom = '\ufeff# PowerShell 安装器';
    result = await get('/install.ps1', {INSTALLERS:{'/install.ps1':bom}});
    assert.equal(result.headers.get('content-type'), 'text/plain; charset=utf-8');
    assert.equal(result.headers.get('cache-control'), 'no-store');
    // Response.text() strips a leading UTF-8 BOM itself; inspect raw bytes.
    assert.deepEqual(Buffer.from(await result.arrayBuffer()), Buffer.from('# PowerShell 安装器'));
    for (const bad of ['/proxy?url=https://example.com', '/releases/download/v0.3.4/evil.exe', '/releases/download/v0.3.4/wocli-music-0.3.3-windows-x86_64.zip', '/releases/download/v0.3.4/wocli-music-0.3.4-linux-x86_64.zip']) {
      assert.equal((await get(bad)).status, 404, bad);
    }
    assert.equal((await get('/', {}, {method:'POST'})).status, 405);
    assert.equal(calls.length, 3);
  } finally { globalThis.fetch = originFetch; cache.clear(); }
});

test('origin failures and forbidden redirects are not cached', async () => {
  try {
    globalThis.fetch = async () => new Response('rate limited', {status:403});
    assert.equal((await get('/api/releases/latest')).status, 502);
    assert.equal(cache.size, 0);
    globalThis.fetch = async () => new Response(null, {status:302, headers:{location:'https://untrusted.example/file'}});
    assert.equal((await get('/api/releases/latest')).status, 502);
    assert.equal(cache.size, 0);
    globalThis.fetch = async () => Response.json({tag_name:'v0.4.0-beta.1', assets:[]});
    assert.equal((await get('/api/releases/latest')).status, 502);
    assert.equal(cache.size, 0);
  } finally {globalThis.fetch = originFetch;}
});

test('GitHub API quota failure falls back to stable-release redirect and caches all platform/source names', async () => {
  let count = 0;
  try {
    globalThis.fetch = async url => {
      count++;
      return url.includes('api.github.com') ? new Response('rate limit', {status:403}) : new Response(null, {status:302, headers:{location:'https://github.com/yhan-sun/wocli-music/releases/tag/v0.3.5'}});
    };
    const metadata = await (await get('/api/releases/latest')).json();
    assert.equal(metadata.tag_name, 'v0.3.5');
    assert.equal(metadata.assets.length, 9);
    assert.equal(metadata.assets.filter(a => a.name.includes('-sources.')).length, 4);
    assert.ok(metadata.assets.every(a => a.browser_download_url.startsWith('https://wocli.yhan.fun/')));
    await Promise.all(tasks.splice(0));
    assert.equal((await get('/api/releases/latest')).status, 200);
    assert.equal(count, 2);
  } finally {globalThis.fetch = originFetch; cache.clear();}
});


test('bundled HTTP installer is UTF-8 without BOM while the PS5.1 source file retains its BOM', async () => {
  const {readFileSync} = await import('node:fs');
  const {execFileSync} = await import('node:child_process');
  execFileSync('python3', ['cloudflare/build.py']);
  const bytes = readFileSync(new URL('../install.ps1', import.meta.url));
  assert.deepEqual(bytes.subarray(0,3), Buffer.from([239,187,191]));
  const {default: bundled} = await import('../dist/cloudflare-worker.mjs');
  const result = await bundled.fetch(request('/install.ps1'), {}, ctx);
  const served = Buffer.from(await result.arrayBuffer());
  assert.deepEqual(served, bytes.subarray(3));
  assert.equal(result.headers.get('content-type'), 'text/plain; charset=utf-8');
  assert.equal(result.headers.get('cache-control'), 'no-store');
});
