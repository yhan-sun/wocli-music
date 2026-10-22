// Only this public repository is reachable; never forward client credentials.
const REPO = 'yhan-sun/wocli-music';
const PLATFORMS = ['windows-x86_64', 'macos-arm64', 'macos-x86_64', 'linux-x86_64'];
const VERSION = 'v[0-9]+\\.[0-9]+\\.[0-9]+';
const ASSET = new RegExp(`^/releases/download/(${VERSION})/(SHA256SUMS|wocli-music-([0-9]+\\.[0-9]+\\.[0-9]+)-(windows-x86_64|macos-arm64|macos-x86_64|linux-x86_64)(-sources)?\\.(zip|tar\\.gz))$`);
const ORIGINS = new Set(['github.com', 'api.github.com', 'release-assets.githubusercontent.com', 'objects.githubusercontent.com']);

function reply(body, status = 200, type = 'text/plain; charset=utf-8', ttl = 0) {
  return new Response(body, {status, headers: {'Content-Type': type, 'Cache-Control': ttl ? `public, max-age=${ttl}` : 'no-store', 'X-Content-Type-Options': 'nosniff'}});
}

function home() {
  return reply(`<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>wocli-music 下载</title><main><h1>wocli-music 下载与更新</h1><p>官方发行版 · Cloudflare 分发</p><ul>${PLATFORMS.map(p => `<li><a href="/download/${p}">${p}</a> · <a href="/download/${p}?sources=1">对应源码</a></li>`).join('')}</ul><h2>Windows PowerShell</h2><pre>irm https://wocli.yhan.fun/install.ps1 | iex</pre><h2>macOS / Linux</h2><pre>curl -fsSL https://wocli.yhan.fun/install.sh | bash</pre><h2>更新</h2><p>新版客户端运行 <code>wocli update</code>。旧版若无法连接 GitHub，可重新运行上方安装命令。</p><p><a href="/api/releases/latest">最新版本与校验文件</a> · <a href="https://github.com/${REPO}">项目源码</a></p></main></html>`, 200, 'text/html; charset=utf-8', 300);
}

async function upstream(url) {
  // Follow GitHub's signed asset redirects on the server, never in the client.
  for (let n = 0; n < 6; n++) {
    if (url.protocol !== 'https:' || !ORIGINS.has(url.hostname)) throw new Error('Invalid origin');
    const result = await fetch(url.href, {
      redirect: 'manual', headers: {'User-Agent': 'wocli-distribution', 'Accept': url.hostname === 'api.github.com' ? 'application/vnd.github+json' : '*/*'},
      signal: AbortSignal.timeout(60000),
    });
    if (![301, 302, 303, 307, 308].includes(result.status)) return result;
    const location = result.headers.get('location');
    await result.body?.cancel();
    if (!location) throw new Error('Missing redirect');
    url = new URL(location, url);
  }
  throw new Error('Too many redirects');
}

async function cached(path, ctx, origin, ttl, transform) {
  // Fixed host/path cache key prevents query-string cache fragmentation.
  const key = new Request(`https://wocli.yhan.fun${path}`);
  const cache = globalThis.caches?.default;
  const hit = await cache?.match(key);
  if (hit) return hit;
  const source = await upstream(new URL(origin));
  if (!source.ok) {
    await source.body?.cancel();
    const error = reply(source.status === 404 ? 'Release or asset not found\n' : 'Release origin unavailable; retry later\n', source.status === 404 ? 404 : 502);
    error.headers.set('X-Wocli-Origin-Status', String(source.status));
    return error;
  }
  let result;
  if (transform) result = await transform(source);
  else {
    const headers = new Headers({'Cache-Control': `public, max-age=${ttl}`, 'X-Content-Type-Options': 'nosniff'});
    for (const header of ['content-type', 'content-length', 'content-disposition', 'etag', 'last-modified']) {
      if (source.headers.has(header)) headers.set(header, source.headers.get(header));
    }
    result = new Response(source.body, {headers});
  }
  if (cache && result.status === 200) ctx.waitUntil(cache.put(key, result.clone()).catch(() => {}));
  return result;
}

async function latest(ctx) {
  let result;
  try { result = await cached('/api/releases/latest', ctx, `https://api.github.com/repos/${REPO}/releases/latest`, 600, async source => {
    const data = await source.json();
    if (!new RegExp(`^${VERSION}$`).test(data.tag_name) || data.draft || data.prerelease || !Array.isArray(data.assets)) throw new Error('Invalid stable release');
    const assets = data.assets.filter(a => ASSET.test(`/releases/download/${data.tag_name}/${a.name}`)).map(a => ({name: a.name, size: a.size, browser_download_url: `https://wocli.yhan.fun/releases/download/${data.tag_name}/${a.name}`}));
    return reply(JSON.stringify({tag_name: data.tag_name, draft: false, prerelease: false, published_at: data.published_at, html_url: 'https://wocli.yhan.fun/', assets}, null, 2), 200, 'application/json; charset=utf-8', 600);
  });
  } catch { result = reply('Release API unavailable\n', 502); }
  if (result.ok || result.status === 404) return result;
  // Public GitHub API limits are shared by many edge users. The stable-release
  // redirect is independent of that API quota; all nine names are guaranteed by CI.
  const redirect = await fetch(`https://github.com/${REPO}/releases/latest`, {
    redirect: 'manual', headers: {'User-Agent': 'wocli-distribution'}, signal: AbortSignal.timeout(30000),
  });
  const location = redirect.headers.get('location');
  await redirect.body?.cancel();
  if (![301,302,303,307,308].includes(redirect.status) || !location) return result;
  const target = new URL(location, `https://github.com/${REPO}/releases/latest`);
  const tag = target.pathname.split('/').pop();
  if (target.origin !== 'https://github.com' || target.pathname !== `/${REPO}/releases/tag/${tag}` || !new RegExp(`^${VERSION}$`).test(tag)) return result;
  const names = ['SHA256SUMS', ...PLATFORMS.flatMap(platform => ['', '-sources'].map(suffix => `wocli-music-${tag.slice(1)}-${platform}${suffix}.${platform.startsWith('windows') ? 'zip' : 'tar.gz'}`))];
  const fallback = reply(JSON.stringify({tag_name:tag, draft:false, prerelease:false, html_url:'https://wocli.yhan.fun/', assets:names.map(name => ({name, browser_download_url:`https://wocli.yhan.fun/releases/download/${tag}/${name}`}))}, null, 2), 200, 'application/json; charset=utf-8', 600);
  const cache = globalThis.caches?.default;
  if (cache) ctx.waitUntil(cache.put(new Request('https://wocli.yhan.fun/api/releases/latest'), fallback.clone()).catch(() => {}));
  return fallback;
}

export default {
  async fetch(request, env, ctx) {
    if (!['GET', 'HEAD'].includes(request.method)) {
      const response = reply('Method not allowed\n', 405); response.headers.set('Allow', 'GET, HEAD'); return response;
    }
    const url = new URL(request.url);
    let response;
    try {
      if (url.pathname === '/') response = home();
      else if (url.pathname === '/install.sh' || url.pathname === '/install.ps1') {
        const script = env.INSTALLERS?.[url.pathname];
        response = script ? reply(script.replace(/^\uFEFF/, ''), 200, 'text/plain; charset=utf-8') : reply('Installer not bundled\n', 503);
      } else if (url.pathname === '/api/releases/latest') response = await latest(ctx);
      else if (url.pathname.startsWith('/download/') && PLATFORMS.includes(url.pathname.slice(10))) {
        const metadata = await latest(ctx);
        if (!metadata.ok) response = metadata;
        else {
          const release = await metadata.json();
          const platform = url.pathname.slice(10);
          const name = `wocli-music-${release.tag_name.slice(1)}-${platform}${url.searchParams.get('sources') === '1' ? '-sources' : ''}.${platform.startsWith('windows') ? 'zip' : 'tar.gz'}`;
          const asset = release.assets.find(a => a.name === name);
          response = asset ? new Response(null, {status: 302, headers: {Location: `/releases/download/${release.tag_name}/${name}`, 'Cache-Control': 'no-store'}}) : reply('Asset not found\n', 404);
        }
      } else {
        const match = ASSET.exec(url.pathname);
        if (!match || (match[3] && match[1] !== `v${match[3]}`) || (match[4] && (match[4].startsWith('windows') !== (match[6] === 'zip')))) response = reply('Not found\n', 404);
        else response = await cached(url.pathname, ctx, `https://github.com/${REPO}${url.pathname}`, 86400);
      }
    } catch { response = reply('Distribution unavailable; retry later\n', 502); }
    // HEAD must not poison the GET cache with an empty response.
    return request.method === 'HEAD' ? new Response(null, {status: response.status, headers: response.headers}) : response;
  },
};
