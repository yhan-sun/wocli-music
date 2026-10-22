// A deliberately small LX host. There is no Node module loader, filesystem or process API.
(() => {
  const handlers = new Map(), requests = new Map(), timers = new Map();
  let next = 1;
  globalThis.__lxState = { ready: null, result: null, done: false, error: '' };
  const bytes = (data, encoding = 'utf8') => {
    if (typeof data !== 'string') return Uint8Array.from(data || []);
    return Uint8Array.from(JSON.parse(__lxUtil('decode', JSON.stringify({data, encoding}))));
  };
  const string = (data, encoding = 'utf8') => __lxUtil('encode', JSON.stringify({data: Array.from(data), encoding}));
  globalThis.btoa = value => string(Uint8Array.from(String(value), c => c.charCodeAt(0)), 'base64');
  globalThis.atob = value => Array.from(bytes(value, 'base64'), b => String.fromCharCode(b)).join('');
  globalThis.TextEncoder = class { encode(value = '') {return bytes(String(value));} };
  globalThis.TextDecoder = class { decode(value = []) {return string(value);} };
  globalThis.Buffer = class Buffer extends Uint8Array {
    static from(value, encoding) { return new Buffer(bytes(value, encoding)); }
    static isBuffer(value) { return value instanceof Buffer; }
    static alloc(size) { if (size > 2 * 1024 * 1024) throw new Error('Buffer 过大'); return new Buffer(size); }
    static concat(items) { const out = Buffer.alloc(items.reduce((n,b) => n+b.length,0)); let at=0; for(const b of items){out.set(b,at);at+=b.length;} return out; }
    toString(encoding = 'utf8') {return string(this, encoding);}
  };
  globalThis.URLSearchParams = class {
    constructor(value = '') {
      const decode = v => {try{return decodeURIComponent(v.replace(/\+/g,' '));}catch{return v;}};
      this.pairs = typeof value === 'string' ? value.replace(/^\?/, '').split('&').filter(Boolean).map(p => {const at=p.indexOf('=');return at<0?[decode(p),'']:[decode(p.slice(0,at)),decode(p.slice(at+1))];}) :
        (value[Symbol.iterator] ? Array.from(value, pair => pair.map(String)) : Object.entries(value).map(([k,v]) => [k,String(v)]));
    }
    append(k,v) {this.pairs.push([String(k),String(v)]);}
    get(k) {return this.pairs.find(p=>p[0]===String(k))?.[1] ?? null;}
    getAll(k) {return this.pairs.filter(p=>p[0]===String(k)).map(p=>p[1]);}
    has(k) {return this.pairs.some(p=>p[0]===String(k));}
    delete(k) {this.pairs=this.pairs.filter(p=>p[0]!==String(k));}
    set(k,v) {const index=this.pairs.findIndex(p=>p[0]===String(k));this.delete(k);this.pairs.splice(index<0?this.pairs.length:index,0,[String(k),String(v)]);}
    sort() {this.pairs.sort((a,b)=>a[0]<b[0]?-1:a[0]>b[0]?1:0);}
    toString() {const encode=v=>encodeURIComponent(v).replace(/%20/g,'+').replace(/[!'()~]/g,c=>'%'+c.charCodeAt(0).toString(16).toUpperCase());return this.pairs.map(([k,v])=>encode(k)+'='+encode(v)).join('&');}
    entries() {return this.pairs[Symbol.iterator]();}
    keys() {return this.pairs.map(p=>p[0])[Symbol.iterator]();}
    values() {return this.pairs.map(p=>p[1])[Symbol.iterator]();}
    forEach(callback,thisArg) {for(const [k,v] of this.pairs)callback.call(thisArg,v,k,this);}
    get size() {return this.pairs.length;}
    [Symbol.iterator]() {return this.entries();}
  };
  globalThis.console = Object.fromEntries(['log', 'warn', 'error', 'info', 'debug'].map(k => [k, () => {}]));
  globalThis.setTimeout = (callback, delay = 0, ...args) => {
    if (timers.size >= 256) throw new Error('音源定时器过多');
    const id = next++; timers.set(id, {callback, args, at: Date.now() + Math.max(0, Number(delay) || 0)}); return id;
  };
  globalThis.clearTimeout = id => timers.delete(id);
  globalThis.setInterval = () => { throw new Error('不支持 setInterval，请使用 setTimeout'); };
  globalThis.clearInterval = globalThis.clearTimeout;
  globalThis.__lxTick = () => {
    for (const [id, timer] of timers) if (timer.at <= Date.now()) {
      timers.delete(id); timer.callback(...timer.args);
    }
  };
  globalThis.__lxTakeRequests = () => JSON.stringify(Array.from(requests, ([id, r]) => ({id, url: r.url, options: r.options})).filter(r => !requests.get(r.id).sent).map(r => {requests.get(r.id).sent = true; return r;}));
  globalThis.__lxReply = (id, error, response) => {
    const r = requests.get(id); if (!r) return;
    requests.delete(id);
    r.callback(error ? new Error(error) : null, response, response?.body);
  };
  globalThis.__lxStart = input => {
    Promise.resolve().then(() => {
      if (!handlers.has('request')) throw new Error('音源未注册 request');
      return handlers.get('request')(input);
    }).then(value => {__lxState.result = value; __lxState.done = true;}, e => {
      __lxState.error = String(e?.message || e); __lxState.done = true;
    });
  };
  globalThis.lx = {
    version: '2.0.0', env: 'desktop', currentScriptInfo: {},
    EVENT_NAMES: Object.freeze({request: 'request', inited: 'inited', updateAlert: 'updateAlert'}),
    on: (event, handler) => { if (typeof handler !== 'function') throw new Error('无效事件处理器'); handlers.set(event, handler); },
    send: (event, value) => { if (event === 'inited') __lxState.ready = value; return Promise.resolve(); },
    request: (url, options, callback) => {
      if (requests.size >= 32) throw new Error('音源请求过多');
      if (typeof options === 'function') { callback = options; options = {}; }
      if (typeof callback !== 'function') throw new Error('request 缺少回调');
      const id = next++; requests.set(id, {url, options: options || {}, callback, sent: false});
      return () => requests.delete(id);
    },
    utils: {
      buffer: {from: bytes, bufToString: string},
      crypto: {
        md5: data => __lxUtil('md5', String(data)),
        randomBytes: n => Uint8Array.from(JSON.parse(__lxUtil('random', String(n)))),
        aesEncrypt: () => {throw new Error('当前 LX 兼容层不支持 AES');},
        rsaEncrypt: () => {throw new Error('当前 LX 兼容层不支持 RSA');},
      },
      zlib: {
        inflate: async data => Uint8Array.from(JSON.parse(__lxUtil('inflate', JSON.stringify(Array.from(data))))),
        deflate: async data => Uint8Array.from(JSON.parse(__lxUtil('deflate', JSON.stringify(Array.from(data))))),
      },
    },
  };
})();
