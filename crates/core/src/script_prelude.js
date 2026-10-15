// Postman-compatible sandbox. `__input_json` is injected by the host.
var __in = JSON.parse(__input_json);
var __out = { tests: [], console: [], errors: [], nextRequestSet: false, nextRequest: null };

function __str(v) {
  if (v === undefined) return 'undefined';
  if (v === null) return 'null';
  if (typeof v === 'object') { try { return JSON.stringify(v, null, 2); } catch (e) { return String(v); } }
  return String(v);
}
function __log(level) {
  return function () { __out.console.push({ level: level, text: Array.prototype.map.call(arguments, __str).join(' ') }); };
}
var console = { log: __log('log'), info: __log('info'), warn: __log('warn'), error: __log('error'), debug: __log('debug') };

function __toStored(v) {
  if (typeof v === 'string') return v;
  if (v === undefined || v === null) return '';
  if (typeof v === 'object') return JSON.stringify(v);
  return String(v);
}
function __replace(s) {
  return String(s).replace(/\{\{\s*([^{}]+?)\s*\}\}/g, function (m, name) {
    var v = pm.variables.get(name);
    return v === undefined ? m : v;
  });
}
function __scope(obj, name) {
  return {
    name: name,
    get: function (k) { return Object.prototype.hasOwnProperty.call(obj, k) ? obj[k] : undefined; },
    set: function (k, v) { obj[k] = __toStored(v); },
    has: function (k) { return Object.prototype.hasOwnProperty.call(obj, k); },
    unset: function (k) { delete obj[k]; },
    clear: function () { for (var k in obj) delete obj[k]; },
    toObject: function () { return Object.assign({}, obj); },
    replaceIn: __replace
  };
}

function __HeaderList(arr) { this._h = (arr || []).map(function (h) { return { key: h.key, value: h.value }; }); }
__HeaderList.prototype._idx = function (n) {
  var l = String(n).toLowerCase();
  for (var i = 0; i < this._h.length; i++) if (this._h[i].key.toLowerCase() === l) return i;
  return -1;
};
__HeaderList.prototype.get = function (n) { var i = this._idx(n); return i < 0 ? undefined : this._h[i].value; };
__HeaderList.prototype.has = function (n) { return this._idx(n) >= 0; };
__HeaderList.prototype.add = function (h) { this._h.push({ key: String(h.key), value: __toStored(h.value) }); };
__HeaderList.prototype.upsert = function (h) {
  var i = this._idx(h.key);
  if (i < 0) this.add(h); else this._h[i].value = __toStored(h.value);
};
__HeaderList.prototype.remove = function (n) { var i; while ((i = this._idx(n)) >= 0) this._h.splice(i, 1); };
__HeaderList.prototype.all = function () { return this._h.slice(); };
__HeaderList.prototype.each = function (fn) { this._h.forEach(fn); };
__HeaderList.prototype.toObject = function () { var o = {}; this._h.forEach(function (h) { o[h.key] = h.value; }); return o; };
__HeaderList.prototype.count = function () { return this._h.length; };

function __Url(raw) { this.raw = raw; }
__Url.prototype.toString = function () { return this.raw; };
__Url.prototype.update = function (s) { this.raw = String(s); };
__Url.prototype.getQueryString = function () { var i = this.raw.indexOf('?'); return i < 0 ? '' : this.raw.slice(i + 1); };
__Url.prototype.addQueryParams = function (ps) {
  var self = this;
  (Array.isArray(ps) ? ps : [ps]).forEach(function (p) {
    var s = typeof p === 'string' ? p : (p.key + '=' + p.value);
    self.raw += (self.raw.indexOf('?') < 0 ? '?' : '&') + s;
  });
};

// ---------- assertions (chai subset) ----------
function AssertionError(msg) { this.name = 'AssertionError'; this.message = msg; }
AssertionError.prototype = Object.create(Error.prototype);
AssertionError.prototype.toString = function () { return 'AssertionError: ' + this.message; };

function __type(v) {
  if (v === null) return 'null';
  if (Array.isArray(v)) return 'array';
  if (v instanceof RegExp) return 'regexp';
  if (v instanceof Date) return 'date';
  return typeof v;
}
function __deepEq(a, b) {
  if (a === b) return true;
  if (typeof a === 'number' && typeof b === 'number' && isNaN(a) && isNaN(b)) return true;
  var ta = __type(a), tb = __type(b);
  if (ta !== tb || (ta !== 'array' && ta !== 'object')) return false;
  if (ta === 'array') {
    if (a.length !== b.length) return false;
    for (var i = 0; i < a.length; i++) if (!__deepEq(a[i], b[i])) return false;
    return true;
  }
  var ka = Object.keys(a), kb = Object.keys(b);
  if (ka.length !== kb.length) return false;
  for (var j = 0; j < ka.length; j++) if (!Object.prototype.hasOwnProperty.call(b, ka[j]) || !__deepEq(a[ka[j]], b[ka[j]])) return false;
  return true;
}
function __fmt(v) { return typeof v === 'string' ? "'" + v + "'" : __str(v); }

function __A(obj, msg) { this._o = obj; this._n = false; this._d = false; this._any = false; this._msg = msg; }
['to', 'be', 'been', 'is', 'that', 'which', 'and', 'has', 'have', 'with', 'at', 'of', 'same', 'but', 'does', 'still', 'all', 'own', 'nested']
  .forEach(function (c) { Object.defineProperty(__A.prototype, c, { get: function () { return this; } }); });
Object.defineProperty(__A.prototype, 'not', { get: function () { this._n = !this._n; return this; } });
Object.defineProperty(__A.prototype, 'deep', { get: function () { this._d = true; return this; } });
Object.defineProperty(__A.prototype, 'any', { get: function () { this._any = true; return this; } });
__A.prototype._assert = function (cond, msg, nmsg) {
  var failed = this._n ? cond : !cond;
  if (failed) {
    var m = this._n ? nmsg : msg;
    throw new AssertionError(this._msg ? this._msg + ': ' + m : m);
  }
  return this;
};
__A.prototype.equal = function (v) {
  if (this._d) return this.eql(v);
  return this._assert(this._o === v, 'expected ' + __fmt(this._o) + ' to equal ' + __fmt(v), 'expected ' + __fmt(this._o) + ' to not equal ' + __fmt(v));
};
__A.prototype.equals = __A.prototype.eq = __A.prototype.equal;
__A.prototype.eql = function (v) {
  return this._assert(__deepEq(this._o, v), 'expected ' + __fmt(this._o) + ' to deeply equal ' + __fmt(v), 'expected ' + __fmt(this._o) + ' to not deeply equal ' + __fmt(v));
};
__A.prototype.a = function (t) {
  var actual = __type(this._o), want = String(t).toLowerCase();
  return this._assert(actual === want, 'expected ' + __fmt(this._o) + ' to be a(n) ' + want + ' but got ' + actual, 'expected ' + __fmt(this._o) + ' not to be a(n) ' + want);
};
__A.prototype.an = __A.prototype.a;
__A.prototype.include = function (v) {
  var o = this._o, ok = false, d = this._d;
  if (typeof o === 'string') ok = o.indexOf(v) >= 0;
  else if (Array.isArray(o)) ok = o.some(function (x) { return d || typeof v === 'object' ? __deepEq(x, v) : x === v; });
  else if (o && typeof o === 'object' && v && typeof v === 'object') ok = Object.keys(v).every(function (k) { return __deepEq(o[k], v[k]); });
  return this._assert(ok, 'expected ' + __fmt(o) + ' to include ' + __fmt(v), 'expected ' + __fmt(o) + ' to not include ' + __fmt(v));
};
__A.prototype.includes = __A.prototype.contain = __A.prototype.contains = __A.prototype.include;
__A.prototype.property = function (name, val) {
  var o = this._o, has = o !== null && o !== undefined && (typeof o === 'object' ? name in o : o[name] !== undefined);
  if (arguments.length < 2) return this._assert(has, 'expected ' + __fmt(o) + ' to have property ' + __fmt(name), 'expected ' + __fmt(o) + ' to not have property ' + __fmt(name));
  var actual = has ? o[name] : undefined;
  var eq = this._d ? __deepEq(actual, val) : actual === val;
  return this._assert(has && eq, 'expected ' + __fmt(o) + ' to have property ' + __fmt(name) + ' of ' + __fmt(val) + ', but got ' + __fmt(actual), 'expected property ' + __fmt(name) + ' to not be ' + __fmt(val));
};
__A.prototype.lengthOf = function (n) {
  var len = this._o == null ? undefined : this._o.length;
  return this._assert(len === n, 'expected length ' + n + ' but got ' + len, 'expected length to not be ' + n);
};
__A.prototype.length = __A.prototype.lengthOf;
__A.prototype.above = function (n) { return this._assert(this._o > n, 'expected ' + __fmt(this._o) + ' to be above ' + n, 'expected ' + __fmt(this._o) + ' to be at most ' + n); };
__A.prototype.below = function (n) { return this._assert(this._o < n, 'expected ' + __fmt(this._o) + ' to be below ' + n, 'expected ' + __fmt(this._o) + ' to be at least ' + n); };
__A.prototype.least = function (n) { return this._assert(this._o >= n, 'expected ' + __fmt(this._o) + ' to be at least ' + n, 'expected ' + __fmt(this._o) + ' to be below ' + n); };
__A.prototype.most = function (n) { return this._assert(this._o <= n, 'expected ' + __fmt(this._o) + ' to be at most ' + n, 'expected ' + __fmt(this._o) + ' to be above ' + n); };
__A.prototype.gt = __A.prototype.greaterThan = __A.prototype.above;
__A.prototype.lt = __A.prototype.lessThan = __A.prototype.below;
__A.prototype.gte = __A.prototype.least;
__A.prototype.lte = __A.prototype.most;
__A.prototype.within = function (a, b) { return this._assert(this._o >= a && this._o <= b, 'expected ' + __fmt(this._o) + ' to be within ' + a + '..' + b, 'expected ' + __fmt(this._o) + ' to not be within ' + a + '..' + b); };
__A.prototype.match = function (re) { return this._assert(re.test(String(this._o)), 'expected ' + __fmt(this._o) + ' to match ' + re, 'expected ' + __fmt(this._o) + ' not to match ' + re); };
__A.prototype.matches = __A.prototype.match;
__A.prototype.string = function (s) { return this._assert(String(this._o).indexOf(s) >= 0, 'expected ' + __fmt(this._o) + ' to contain ' + __fmt(s), 'expected ' + __fmt(this._o) + ' to not contain ' + __fmt(s)); };
__A.prototype.oneOf = function (arr) { var o = this._o; return this._assert(arr.some(function (x) { return __deepEq(x, o); }), 'expected ' + __fmt(o) + ' to be one of ' + __fmt(arr), 'expected ' + __fmt(o) + ' to not be one of ' + __fmt(arr)); };
__A.prototype.keys = function () {
  var want = Array.isArray(arguments[0]) ? arguments[0] : Array.prototype.slice.call(arguments);
  var have = Object.keys(this._o || {});
  var ok = this._any ? want.some(function (k) { return have.indexOf(k) >= 0; }) : want.every(function (k) { return have.indexOf(k) >= 0; });
  return this._assert(ok, 'expected ' + __fmt(this._o) + ' to have keys ' + __fmt(want), 'expected ' + __fmt(this._o) + ' to not have keys ' + __fmt(want));
};
__A.prototype.key = __A.prototype.keys;
__A.prototype.members = function (arr) {
  var o = this._o || [];
  var ok = arr.length === o.length && arr.every(function (x) { return o.some(function (y) { return __deepEq(x, y); }); });
  return this._assert(ok, 'expected ' + __fmt(o) + ' to have the same members as ' + __fmt(arr), 'expected ' + __fmt(o) + ' to not have the same members as ' + __fmt(arr));
};
__A.prototype.instanceof = function (C) { return this._assert(this._o instanceof C, 'expected instance', 'expected not an instance'); };
__A.prototype.instanceOf = __A.prototype.instanceof;
__A.prototype.satisfy = function (fn) { return this._assert(!!fn(this._o), 'expected ' + __fmt(this._o) + ' to satisfy predicate', 'expected ' + __fmt(this._o) + ' to not satisfy predicate'); };
function __prop(name, check, what) {
  Object.defineProperty(__A.prototype, name, {
    get: function () { return this._assert(check(this._o), 'expected ' + __fmt(this._o) + ' to be ' + what, 'expected ' + __fmt(this._o) + ' to not be ' + what); }
  });
}
__prop('ok', function (o) { return !!o; }, 'truthy');
__prop('true', function (o) { return o === true; }, 'true');
__prop('false', function (o) { return o === false; }, 'false');
__prop('null', function (o) { return o === null; }, 'null');
__prop('undefined', function (o) { return o === undefined; }, 'undefined');
__prop('NaN', function (o) { return typeof o === 'number' && isNaN(o); }, 'NaN');
__prop('exist', function (o) { return o !== null && o !== undefined; }, 'existing');
__prop('empty', function (o) {
  if (o == null) return true;
  if (typeof o === 'string' || Array.isArray(o)) return o.length === 0;
  if (typeof o === 'object') return Object.keys(o).length === 0;
  return false;
}, 'empty');

// ---------- pm ----------
var __locals = __in.locals, __env = __in.env, __coll = __in.collection, __glob = __in.globals, __data = __in.data;
var pm = {};
pm.environment = __scope(__env, __in.envName);
pm.collectionVariables = __scope(__coll, 'collection');
pm.globals = __scope(__glob, 'globals');
pm.iterationData = __scope(__data, 'data');
pm.variables = __scope(__locals, 'local');
pm.variables.get = function (k) {
  var layers = [__locals, __data, __env, __coll, __glob];
  for (var i = 0; i < layers.length; i++) if (Object.prototype.hasOwnProperty.call(layers[i], k)) return layers[i][k];
  return undefined;
};
pm.variables.has = function (k) { return pm.variables.get(k) !== undefined; };
pm.variables.toObject = function () { return Object.assign({}, __glob, __coll, __env, __data, __locals); };
pm.info = __in.info;
pm.request = {
  method: __in.request.method,
  url: new __Url(__in.request.url),
  headers: new __HeaderList(__in.request.headers),
  body: { raw: __in.request.body, toString: function () { return this.raw; } }
};
pm.expect = function (v, msg) { return new __A(v, msg); };
pm.test = function (name, fn) {
  try {
    fn();
    __out.tests.push({ name: String(name), passed: true, error: '' });
  } catch (e) {
    __out.tests.push({ name: String(name), passed: false, error: (e && e.message) ? e.message : String(e) });
  }
};
pm.test.skip = function (name) { __out.tests.push({ name: String(name) + ' (skipped)', passed: true, error: '' }); };
pm.setNextRequest = function (n) { __out.nextRequestSet = true; __out.nextRequest = n === null ? null : String(n); };
pm.execution = { setNextRequest: pm.setNextRequest, skipRequest: function () { __out.skip = true; } };
pm.sendRequest = function () { throw new Error('pm.sendRequest is not supported in Tessera yet'); };

if (__in.response) {
  var __r = __in.response;
  var __headers = new __HeaderList(__r.headers);
  var __parsed;
  pm.response = {
    code: __r.code,
    status: __r.status,
    responseTime: __r.responseTime,
    responseSize: __r.responseSize,
    headers: __headers,
    text: function () { return __r.body; },
    json: function () { if (__parsed === undefined) __parsed = JSON.parse(__r.body); return __parsed; }
  };
  var __fail = function (m) { throw new AssertionError(m); };
  var __have = {
    status: function (c) {
      if (typeof c === 'number') { if (__r.code !== c) __fail('expected response to have status code ' + c + ' but got ' + __r.code); }
      else if (String(__r.status).toLowerCase() !== String(c).toLowerCase()) __fail("expected response to have status reason '" + c + "' but got '" + __r.status + "'");
    },
    header: function (n, v) {
      if (!__headers.has(n)) __fail("expected response to have header '" + n + "'");
      if (arguments.length > 1 && __headers.get(n) !== v) __fail("expected header '" + n + "' to be '" + v + "' but got '" + __headers.get(n) + "'");
    },
    body: function (s) {
      if (s instanceof RegExp) { if (!s.test(__r.body)) __fail('expected response body to match ' + s); }
      else if (__r.body !== s) __fail('expected response body to equal ' + __fmt(s));
    },
    jsonBody: function (path, val) {
      var j;
      try { j = pm.response.json(); } catch (e) { __fail('expected response body to be valid JSON'); }
      if (arguments.length === 0) return;
      if (typeof path === 'object') { if (!__deepEq(j, path)) __fail('expected JSON body to equal ' + __fmt(path)); return; }
      var cur = j, parts = String(path).split('.');
      for (var i = 0; i < parts.length; i++) {
        if (cur == null || !(parts[i] in Object(cur))) __fail("expected JSON body to have path '" + path + "'");
        cur = cur[parts[i]];
      }
      if (arguments.length > 1 && !__deepEq(cur, val)) __fail("expected '" + path + "' to be " + __fmt(val) + ' but got ' + __fmt(cur));
    }
  };
  var __be = {};
  var __beProp = function (name, check, what) {
    Object.defineProperty(__be, name, { get: function () { if (!check()) __fail('expected response to be ' + what + ' but got ' + __r.code); return true; } });
  };
  __beProp('ok', function () { return __r.code >= 200 && __r.code < 300; }, 'ok (2xx)');
  __beProp('success', function () { return __r.code >= 200 && __r.code < 300; }, 'success (2xx)');
  __beProp('error', function () { return __r.code >= 400; }, 'an error (4xx/5xx)');
  __beProp('clientError', function () { return __r.code >= 400 && __r.code < 500; }, 'a client error (4xx)');
  __beProp('serverError', function () { return __r.code >= 500; }, 'a server error (5xx)');
  __beProp('notFound', function () { return __r.code === 404; }, 'not found (404)');
  __beProp('unauthorized', function () { return __r.code === 401; }, 'unauthorized (401)');
  __beProp('forbidden', function () { return __r.code === 403; }, 'forbidden (403)');
  __beProp('badRequest', function () { return __r.code === 400; }, 'a bad request (400)');
  __beProp('json', function () { try { pm.response.json(); return true; } catch (e) { return false; } }, 'JSON');
  pm.response.to = { have: __have, be: __be };
  pm.response.to.have.be = __be;
  pm.cookies = {
    get: function (n) { for (var i = 0; i < __r.cookies.length; i++) if (__r.cookies[i].name === n) return __r.cookies[i].value; return undefined; },
    has: function (n) { return pm.cookies.get(n) !== undefined; },
    toObject: function () { var o = {}; __r.cookies.forEach(function (c) { o[c.name] = c.value; }); return o; }
  };
  // Legacy sandbox globals.
  var responseBody = __r.body, responseCode = { code: __r.code, name: __r.status }, responseTime = __r.responseTime;
  var responseHeaders = __headers.toObject();
}

var tests = {};
var postman = {
  setEnvironmentVariable: function (k, v) { pm.environment.set(k, v); },
  getEnvironmentVariable: function (k) { return pm.environment.get(k); },
  clearEnvironmentVariable: function (k) { pm.environment.unset(k); },
  setGlobalVariable: function (k, v) { pm.globals.set(k, v); },
  getGlobalVariable: function (k) { return pm.globals.get(k); },
  clearGlobalVariable: function (k) { pm.globals.unset(k); },
  setNextRequest: pm.setNextRequest
};

var __b64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=';
function btoa(input) {
  var str = String(input), out = '';
  for (var i = 0; i < str.length; i += 3) {
    var a = str.charCodeAt(i), b = str.charCodeAt(i + 1), c = str.charCodeAt(i + 2);
    var n = (a << 16) | ((b || 0) << 8) | (c || 0);
    out += __b64.charAt((n >> 18) & 63) + __b64.charAt((n >> 12) & 63) +
      (isNaN(b) ? '=' : __b64.charAt((n >> 6) & 63)) + (isNaN(c) ? '=' : __b64.charAt(n & 63));
  }
  return out;
}
function atob(input) {
  var str = String(input).replace(/[^A-Za-z0-9+/]/g, ''), out = '';
  for (var i = 0; i < str.length; i += 4) {
    var n = (__b64.indexOf(str.charAt(i)) << 18) | (__b64.indexOf(str.charAt(i + 1)) << 12) |
      ((__b64.indexOf(str.charAt(i + 2)) & 63) << 6) | (__b64.indexOf(str.charAt(i + 3)) & 63);
    out += String.fromCharCode((n >> 16) & 255);
    if (i + 2 < str.length) out += String.fromCharCode((n >> 8) & 255);
    if (i + 3 < str.length) out += String.fromCharCode(n & 255);
  }
  return out;
}

function __finish() {
  for (var k in tests) __out.tests.push({ name: k, passed: !!tests[k], error: tests[k] ? '' : 'legacy test failed' });
  return JSON.stringify({
    env: __env, collection: __coll, globals: __glob, locals: __locals,
    tests: __out.tests, console: __out.console, errors: __out.errors,
    nextRequestSet: __out.nextRequestSet, nextRequest: __out.nextRequest, skip: !!__out.skip,
    request: { method: String(pm.request.method), url: String(pm.request.url), headers: pm.request.headers.all() }
  });
}
