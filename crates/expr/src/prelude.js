// CFA4 expression runtime: a small After Effects-style expression object model and the global
// functions, written from the public Expression Language Reference (behaviour only), ported from
// the EC-donor (#11) expression crate (MIT OR Apache-2.0; see docs/licensing.md §5).
//
// Natives (Rust): __h(op, ...) host request; __wig(...) wiggle offsets; __noise(x, y, z);
// __rnd(...) seeded random; __nest(src) source-nesting check. This file is evaluated once per
// runtime and is NOT passed through the array-maths rewriter, so it uses plain JS operators.
//
// Determinism: nothing here reads a clock or an unseeded random source.

var time = 0;
var value = 0;
var index = 0;
var frameDuration = 1 / 30;
var thisProperty = null;
var __S = null;
var __rc = 0;
var __seed = 0;
var __timeless = false;

// Request ops (must match runtime.rs).
var __INFO = 0, __VALUE = 1;

function __begin(t, v, path, uid, idx, fd) {
  time = t;
  value = v;
  index = idx;
  frameDuration = fd;
  __S = { path: path, uid: uid };
  __rc = 0;
  __seed = 0;
  __timeless = false;
  thisProperty = new Prop(path);
}

function __finish(r) {
  // Rethrow from this frame so the runtime's stack unwinds (see `Runtime::script`).
  try {
    return __result(r);
  } catch (e) {
    throw e;
  }
}

function __result(r) {
  r = __v(r);
  if (Array.isArray(r)) return r.map(function (x) { return __v(x); });
  return r;
}

function __err(msg) {
  throw new Error(msg);
}

// ---------------------------------------------------------------------------------------------
// Array maths (targets of the operator rewrite).

function __v(x) {
  if (x !== null && typeof x === 'object' && x.__isProp === true) return x.value;
  return x;
}

function __bin(a, b, f) {
  a = __v(a);
  b = __v(b);
  var aa = Array.isArray(a), ba = Array.isArray(b), i, r;
  if (aa && ba) {
    var n = Math.max(a.length, b.length);
    r = new Array(n);
    for (i = 0; i < n; i++) r[i] = f(i < a.length ? a[i] : 0, i < b.length ? b[i] : 0);
    return r;
  }
  if (aa && (typeof b === 'number' || typeof b === 'boolean')) {
    r = new Array(a.length);
    for (i = 0; i < a.length; i++) r[i] = f(a[i], b);
    return r;
  }
  if (ba && (typeof a === 'number' || typeof a === 'boolean')) {
    r = new Array(b.length);
    for (i = 0; i < b.length; i++) r[i] = f(a, b[i]);
    return r;
  }
  return f(a, b);
}

function __fadd(x, y) { return x + y; }
function __fsub(x, y) { return x - y; }
function __fmul(x, y) { return x * y; }
function __fdiv(x, y) { return x / y; }

function __add(a, b) {
  if (typeof a === 'number' && typeof b === 'number') return a + b;
  var x = __v(a), y = __v(b);
  if (typeof x === 'string' || typeof y === 'string') return x + y;
  return __bin(x, y, __fadd);
}
function __sub(a, b) {
  if (typeof a === 'number' && typeof b === 'number') return a - b;
  return __bin(a, b, __fsub);
}
function __mul(a, b) {
  if (typeof a === 'number' && typeof b === 'number') return a * b;
  return __bin(a, b, __fmul);
}
function __div(a, b) {
  if (typeof a === 'number' && typeof b === 'number') return a / b;
  return __bin(a, b, __fdiv);
}
function __neg(a) {
  if (typeof a === 'number') return -a;
  a = __v(a);
  if (Array.isArray(a)) return a.map(function (x) { return -x; });
  return -a;
}

function __mod(a, d) { return ((a % d) + d) % d; }

// ---------------------------------------------------------------------------------------------
// Vector maths.

function add(a, b) { return __bin(a, b, __fadd); }
function sub(a, b) { return __bin(a, b, __fsub); }
function mul(a, b) { return __bin(a, b, __fmul); }
function div(a, b) { return __bin(a, b, __fdiv); }

function clamp(v, a, b) {
  v = __v(v); a = __v(a); b = __v(b);
  var lo = function (x, y) { return Math.min(Math.max(x, Math.min(y[0], y[1])), Math.max(y[0], y[1])); };
  if (Array.isArray(v)) {
    return v.map(function (x, i) {
      return lo(x, [Array.isArray(a) ? a[i] : a, Array.isArray(b) ? b[i] : b]);
    });
  }
  return lo(v, [a, b]);
}

function dot(a, b) {
  a = __v(a); b = __v(b);
  var s = 0;
  for (var i = 0; i < Math.min(a.length, b.length); i++) s += a[i] * b[i];
  return s;
}

function cross(a, b) {
  a = __v(a); b = __v(b);
  var a2 = a[2] || 0, b2 = b[2] || 0;
  return [a[1] * b2 - a2 * b[1], a2 * b[0] - a[0] * b2, a[0] * b[1] - a[1] * b[0]];
}

function length(a, b) {
  a = __v(a);
  if (b !== undefined) a = sub(a, __v(b));
  if (!Array.isArray(a)) return Math.abs(a);
  var s = 0;
  for (var i = 0; i < a.length; i++) s += a[i] * a[i];
  return Math.sqrt(s);
}

function normalize(a) {
  a = __v(a);
  var l = length(a);
  return l === 0 ? a.map(function () { return 0; }) : div(a, l);
}

function degreesToRadians(d) { return d * Math.PI / 180; }
function radiansToDegrees(r) { return r * 180 / Math.PI; }

// ---------------------------------------------------------------------------------------------
// Interpolation.

function __interp(shape, args) {
  var t, tMin, tMax, v1, v2;
  if (args.length <= 3) {
    t = args[0]; tMin = 0; tMax = 1; v1 = args[1]; v2 = args[2];
  } else {
    t = args[0]; tMin = args[1]; tMax = args[2]; v1 = args[3]; v2 = args[4];
  }
  t = __v(t); v1 = __v(v1); v2 = __v(v2);
  var u;
  if (tMax === tMin) u = t >= tMax ? 1 : 0;
  else u = (t - tMin) / (tMax - tMin);
  u = Math.max(0, Math.min(1, u));
  u = shape(u);
  if (Array.isArray(v1) || Array.isArray(v2)) return add(v1, mul(sub(v2, v1), u));
  return v1 + (v2 - v1) * u;
}

function __lin(u) { return u; }
function __ease(u) { return u * u * (3 - 2 * u); }
function __easeIn(u) { return u * u * (2 - u); }
function __easeOut(u) { var w = 1 - u; return 1 - w * w * (2 - w); }

function linear() { return __interp(__lin, arguments); }
function ease() { return __interp(__ease, arguments); }
function easeIn() { return __interp(__easeIn, arguments); }
function easeOut() { return __interp(__easeOut, arguments); }

// ---------------------------------------------------------------------------------------------
// Random numbers and noise.

function seedRandom(offset, timeless) {
  __seed = offset;
  __timeless = !!timeless;
}

function __r01() {
  return __rnd(index, __S.uid, __seed, __timeless, time, __rc++);
}

function __scaleRand(r, args) {
  if (args.length === 0) return r();
  var a = __v(args[0]);
  if (args.length === 1) {
    if (Array.isArray(a)) return a.map(function (m) { return r() * m; });
    return r() * a;
  }
  var b = __v(args[1]);
  if (Array.isArray(a) || Array.isArray(b)) {
    var n = Math.max(Array.isArray(a) ? a.length : 1, Array.isArray(b) ? b.length : 1), out = [];
    for (var i = 0; i < n; i++) {
      var lo = Array.isArray(a) ? (a[i] || 0) : a, hi = Array.isArray(b) ? (b[i] || 0) : b;
      out.push(lo + r() * (hi - lo));
    }
    return out;
  }
  return a + r() * (b - a);
}

function random() { return __scaleRand(__r01, arguments); }

function __gauss01() {
  var u1 = Math.max(__r01(), 1e-12), u2 = __r01();
  var z = Math.sqrt(-2 * Math.log(u1)) * Math.cos(2 * Math.PI * u2);
  return 0.5 + z * 0.304;
}

function gaussRandom() { return __scaleRand(__gauss01, arguments); }

function noise(x) {
  x = __v(x);
  if (Array.isArray(x)) return __noise(x[0] || 0, x[1] || 0, x[2] || 0);
  return __noise(x, 0, 0);
}

// ---------------------------------------------------------------------------------------------
// Time conversion.

function timeToFrames(t, fps) {
  if (t === undefined) t = time;
  if (fps === undefined) fps = 1 / frameDuration;
  return Math.floor(t * fps + 1e-6);
}

function framesToTime(frames, fps) {
  if (fps === undefined) fps = 1 / frameDuration;
  return frames / fps;
}

function posterizeTime(fps) {
  if (!(fps > 0)) return;
  var t = Math.floor(time * fps + 1e-6) / fps;
  if (t !== time) {
    var v = thisProperty.valueAtTime(t);
    time = t;
    value = v;
  }
}

// ---------------------------------------------------------------------------------------------
// Properties: `thisProperty` and `prop(path)`.

function Prop(path) {
  this.__p = path;
  this.__i = null;
  this.__self = __S !== null && path === __S.path;
}

function __avg(vals) {
  var s = vals[0];
  for (var i = 1; i < vals.length; i++) s = add(s, vals[i]);
  return div(s, vals.length);
}

function __loop(p, out, type, n, dur) {
  type = type === undefined ? 'cycle' : String(type).toLowerCase();
  var times = p.__info[0], N = times.length, t = time;
  if (N === 0) return p.value;
  var tStart, tEnd, h = 1e-3, k, ph;
  if (out) {
    tEnd = times[N - 1];
    if (t <= tEnd) return p.value;
    if (dur !== undefined && dur > 0) tStart = tEnd - dur;
    else if (n > 0 && N - 1 - n >= 0) tStart = times[N - 1 - n];
    else tStart = times[0];
  } else {
    tStart = times[0];
    if (t >= tStart) return p.value;
    if (dur !== undefined && dur > 0) tEnd = tStart + dur;
    else if (n > 0 && n < N) tEnd = times[n];
    else tEnd = times[N - 1];
  }
  var d = tEnd - tStart;
  if (type === 'continue') {
    if (out) {
      var ve = p.valueAtTime(tEnd);
      return add(ve, mul(div(sub(ve, p.valueAtTime(tEnd - h)), h), t - tEnd));
    }
    var vs = p.valueAtTime(tStart);
    return add(vs, mul(div(sub(p.valueAtTime(tStart + h), vs), h), t - tStart));
  }
  if (!(d > 0)) return p.value;
  ph = __mod(t - tStart, d);
  if (type === 'pingpong') {
    if (out) {
      k = Math.floor((t - tStart) / d);
      return p.valueAtTime(k % 2 === 1 ? tEnd - ph : tStart + ph);
    }
    k = Math.floor((tEnd - t) / d);
    return p.valueAtTime(k % 2 === 1 ? tStart + __mod(tStart - t, d) : tEnd - __mod(tStart - t, d));
  }
  if (type === 'offset') {
    k = Math.floor((t - tStart) / d);
    return add(p.valueAtTime(tStart + ph), mul(sub(p.valueAtTime(tEnd), p.valueAtTime(tStart)), k));
  }
  return p.valueAtTime(tStart + ph);
}

Prop.prototype = {
  __isProp: true,
  // [keyTimes, keyValues]
  get __info() { return this.__i || (this.__i = __h(__INFO, this.__p)); },
  get numKeys() { return this.__info[0].length; },
  get name() { return this.__p; },
  get value() { return this.__self ? value : this.valueAtTime(time); },
  valueAtTime: function (t) {
    t = __v(t);
    if (this.__self && t === time) return value;
    return __h(__VALUE, this.__p, t);
  },
  velocityAtTime: function (t) {
    var h = 1e-3;
    return div(sub(this.valueAtTime(t + h), this.valueAtTime(t - h)), 2 * h);
  },
  get velocity() { return this.velocityAtTime(time); },
  speedAtTime: function (t) { return length(this.velocityAtTime(t)); },
  get speed() { return this.speedAtTime(time); },
  key: function (n) {
    var info = this.__info, N = info[0].length;
    if (!(n >= 1 && n <= N)) __err('Key index ' + n + ' out of range (property has ' + N + ' keys)');
    n = Math.floor(n);
    return { time: info[0][n - 1], value: info[1][n - 1], index: n };
  },
  nearestKey: function (t) {
    var times = this.__info[0];
    if (times.length === 0) __err('Property has no keyframes');
    var best = 0;
    for (var i = 1; i < times.length; i++) if (Math.abs(times[i] - t) < Math.abs(times[best] - t)) best = i;
    return this.key(best + 1);
  },
  wiggle: function (freq, amp, octaves, ampMult, t) {
    if (octaves === undefined) octaves = 1;
    if (ampMult === undefined) ampMult = 0.5;
    if (t === undefined) t = time;
    var base = this.valueAtTime(t);
    var dims = Array.isArray(base) ? base.length : 1;
    var off = __wig(index, this.__self ? __S.uid : 0, this.__p, dims, __v(freq), __v(amp), octaves, ampMult, t);
    if (!Array.isArray(base)) return base + off[0];
    var r = base.slice();
    for (var i = 0; i < r.length; i++) r[i] += off[i];
    return r;
  },
  temporalWiggle: function (freq, amp, octaves, ampMult, t) {
    if (octaves === undefined) octaves = 1;
    if (ampMult === undefined) ampMult = 0.5;
    if (t === undefined) t = time;
    var off = __wig(index, this.__self ? __S.uid : 0, this.__p, 1, __v(freq), __v(amp), octaves, ampMult, t);
    return this.valueAtTime(t + off[0]);
  },
  smooth: function (width, samples, t) {
    if (width === undefined) width = 0.2;
    if (samples === undefined) samples = 5;
    if (t === undefined) t = time;
    samples = Math.min(Math.max(1, Math.round(samples)), 64);
    if (samples === 1) return this.valueAtTime(t);
    var vals = [];
    for (var i = 0; i < samples; i++) vals.push(this.valueAtTime(t - width / 2 + width * i / (samples - 1)));
    return __avg(vals);
  },
  loopOut: function (type, n) { return __loop(this, true, type, n || 0); },
  loopIn: function (type, n) { return __loop(this, false, type, n || 0); },
  loopOutDuration: function (type, d) { return __loop(this, true, type, 0, d === undefined ? 0 : d); },
  loopInDuration: function (type, d) { return __loop(this, false, type, 0, d === undefined ? 0 : d); },
  valueOf: function () { return this.value; },
  toString: function () { return String(this.value); },
};

function prop(path) { return new Prop(String(path)); }

function wiggle(f, a, o, m, t) { return thisProperty.wiggle(f, a, o, m, t); }
function temporalWiggle(f, a, o, m, t) { return thisProperty.temporalWiggle(f, a, o, m, t); }
function smooth(w, s, t) { return thisProperty.smooth(w, s, t); }
function loopOut(type, n) { return thisProperty.loopOut(type, n); }
function loopIn(type, n) { return thisProperty.loopIn(type, n); }
function loopOutDuration(type, d) { return thisProperty.loopOutDuration(type, d); }
function loopInDuration(type, d) { return thisProperty.loopInDuration(type, d); }
function valueAtTime(t) { return thisProperty.valueAtTime(t); }
function velocityAtTime(t) { return thisProperty.velocityAtTime(t); }
function speedAtTime(t) { return thisProperty.speedAtTime(t); }
function key(n) { return thisProperty.key(n); }
function nearestKey(t) { return thisProperty.nearestKey(t); }

// ---------------------------------------------------------------------------------------------
// Stack safety. A stack overflow aborts the whole process, so nothing a script does may nest
// native code without bound. The expression text is checked for nesting before it is compiled;
// source handed to `eval` and the function constructors at run time is checked by `__nest`.
// The natives that walk nested values recursively (`join`, which `toString` and string
// conversion use, `toLocaleString`) run through a wrapper that counts nesting levels explicitly
// (a depth counter, not a stack address, so the limit is the same on every thread and build).
// `JSON.stringify` measures the value's depth before serializing. `Proxy` (chains of proxies nest
// without any script frame) and `Reflect` are not available to expressions.
(function () {
  var nest = __nest, apply = Reflect.apply, construct = Reflect.construct;
  var define = Object.defineProperty, describe = Object.getOwnPropertyDescriptor, proto = Object.getPrototypeOf;
  var MAX_DEPTH = 40, depth = 0;
  function replace(obj, name, f) {
    var d = describe(obj, name);
    define(obj, name, { value: f, writable: d.writable, enumerable: d.enumerable, configurable: true });
  }
  function guard(obj, name) {
    var f = obj[name];
    replace(obj, name, function () {
      if (depth >= MAX_DEPTH) throw new RangeError('value is nested too deeply');
      depth++;
      try {
        return apply(f, this, arguments);
      } finally {
        depth--;
      }
    });
  }

  // Indirect on purpose: a wrapper can't make a direct eval, so `eval` sees globals and `var`s
  // but not `let`/`const` locals of the expression.
  var evalJs = eval;
  eval = function (x) {
    if (typeof x === 'string') nest(x);
    return evalJs(x);
  };
  // `Function` and the generator / async variants (reachable as `(function* () {}).constructor`).
  function dynamic(F) {
    var w = function () {
      var args = [];
      for (var i = 0; i < arguments.length; i++) args.push(String(arguments[i]));
      nest(args.slice(0, -1).join(','));
      nest(args.length ? args[args.length - 1] : '');
      return construct(F, args);
    };
    define(w, 'name', { value: F.name, configurable: true });
    w.prototype = F.prototype;
    replace(F.prototype, 'constructor', w);
    return w;
  }
  Function = dynamic(Function);
  dynamic(proto(function* () {}).constructor);
  dynamic(proto(async function () {}).constructor);
  dynamic(proto(async function* () {}).constructor);

  // Native allocation bombs are not covered by the loop limit or the instruction budget: refuse
  // the constructors and string builders that take a size (`Array(1e9)`, `'x'.repeat(1e9)`).
  var MAX_SIZE = 1 << 20;
  var ArrayJs = Array;
  var ArrayGuard = function () {
    var a = arguments;
    if (a.length === 1 && typeof a[0] === 'number' && a[0] > MAX_SIZE) throw new RangeError('array too large');
    return construct(ArrayJs, a);
  };
  ArrayGuard.prototype = ArrayJs.prototype;
  ['isArray', 'from', 'of'].forEach(function (k) { define(ArrayGuard, k, { value: ArrayJs[k], writable: true, configurable: true }); });
  define(ArrayGuard, 'name', { value: 'Array', configurable: true });
  replace(ArrayJs.prototype, 'constructor', ArrayGuard);
  Array = ArrayGuard;
  ['repeat', 'padStart', 'padEnd'].forEach(function (k) {
    var f = String.prototype[k];
    replace(String.prototype, k, function (n) {
      var len = Number(n);
      if (len > MAX_SIZE || len * String(this).length > MAX_SIZE) throw new RangeError('string too large');
      return apply(f, this, arguments);
    });
  });

  guard(Array.prototype, 'join');
  guard(Array.prototype, 'toLocaleString');
  var flat = Array.prototype.flat;
  replace(Array.prototype, 'flat', function (d) {
    var n = d === undefined ? 1 : Number(d);
    return apply(flat, this, [n > 32 ? 32 : n]);
  });

  // The deepest nesting of `v`, found with an explicit stack and a node budget; anything deeper
  // than MAX_DEPTH (or too large to walk) is refused. A cycle is as deep as the budget allows.
  var stringify = JSON.stringify;
  replace(JSON, 'stringify', function (v, replacer, space) {
    var stack = [[v, 0]], seen = 0;
    while (stack.length > 0) {
      var top = stack.pop(), o = top[0], dpt = top[1];
      if (o === null || typeof o !== 'object') continue;
      if (dpt >= MAX_DEPTH || ++seen > 100000) throw new RangeError('value is nested too deeply');
      var ks = Object.keys(o);
      for (var i = 0; i < ks.length; i++) stack.push([o[ks[i]], dpt + 1]);
    }
    return apply(stringify, JSON, [v, replacer, space]);
  });

  delete globalThis.Proxy;
  delete globalThis.Reflect;
})();

// ---------------------------------------------------------------------------------------------
// The runtime runs this file once and keeps it for the thread's later evaluations, so an
// expression can't change the object model or the internals: the prototypes are frozen, and the
// `__` globals and the constructors can't be reassigned. The `__` state above is set again by
// `__begin` in each evaluation.

[Prop].forEach(function (f) {
  Object.freeze(f.prototype);
  Object.freeze(f);
});
(function () {
  var perEvaluation = { __S: true, __rc: true, __seed: true, __timeless: true };
  var model = { Prop: true };
  Object.getOwnPropertyNames(globalThis).forEach(function (k) {
    if (!(k.slice(0, 2) === '__' && !perEvaluation[k]) && !model[k]) return;
    if ('value' in Object.getOwnPropertyDescriptor(globalThis, k)) Object.defineProperty(globalThis, k, { writable: false, configurable: false });
  });
})();

// ---------------------------------------------------------------------------------------------
// One runtime serves every expression on a thread, and user code runs as a script at global
// scope, so `linear = …` or a top-level `var clamp` would otherwise rebind a prelude name for
// every later expression, and for every later project in the session. The block above
// closes the `__` globals, the model constructors and the prelude's own prototypes; what is left
// is the rest of the global surface: the prelude's own helpers (`linear`, `clamp`, `wiggle`), the
// JavaScript built-ins that an expression and the prelude both call, and the prototypes those
// hang off. A non-writable, non-configurable binding makes such a write a silent no-op in sloppy
// mode, and `var` leaves an existing global property alone. A name the user introduces is not
// frozen, so an expression that declares its own functions still works.
(function () {
  var g = globalThis;
  // Every intrinsic this block needs, read once before the first prototype is rewritten. The
  // block turns `Array.prototype.concat` and `forEach` into accessors as it goes, so reaching for
  // them through a prototype part-way down the list would be reaching for something it has just
  // changed under itself: the loops below index arrays directly for the same reason.
  var describe = Object.getOwnPropertyDescriptor, define = Object.defineProperty;
  var getNames = Object.getOwnPropertyNames, getSyms = Object.getOwnPropertySymbols;
  var freeze = Object.freeze, isFrozen = Object.isFrozen, isExtensible = Object.isExtensible;
  var protoOf = Object.getPrototypeOf;

  var lock = function (n) {
    var d = describe(g, n);
    if (!d || !d.writable) return;
    define(g, n, { value: d.value, writable: false, enumerable: d.enumerable, configurable: false });
  };

  // `name` and `message` on an error's prototype are the two names the repair below has to leave
  // alone. The host renders a thrown error by reading both straight off the object, with no
  // context to call a getter with, and it has no way to run one safely either: the time budget is
  // checked between polls of the evaluation, and formatting happens after that returns, so a
  // getter of the expression's own would run outside it — bounded by the loop and recursion limits
  // at some hundreds of millions of iterations, which is a hang. Left as data properties they are
  // frozen rather than repaired, so `e.name = 'MyError'` on an expression's own error is dropped
  // in silence. That is the cost; an unbounded getter in the failure path is not worth paying to
  // avoid it.
  var HOST_READ = ['name', 'message'];
  var hostRead = [];
  var errorNames = ['Error', 'EvalError', 'RangeError', 'ReferenceError', 'SyntaxError', 'TypeError',
    'URIError', 'AggregateError'];
  for (var e = 0; e < errorNames.length; e++) {
    var C = g[errorNames[e]];
    if (typeof C === 'function') {
      var cd = describe(C, 'prototype');
      if (cd && 'value' in cd) hostRead.push(cd.value);
    }
  }
  var readByHost = function (p, n) {
    for (var i = 0; i < HOST_READ.length; i++) {
      if (n !== HOST_READ[i]) continue;
      for (var j = 0; j < hostRead.length; j++) if (hostRead[j] === p) return true;
    }
    return false;
  };

  // Freezing a prototype costs an expression the right to shadow an inherited name on its own
  // object: once the inherited data property is non-writable, `o.toString = …` is dropped in
  // sloppy mode rather than creating an own property — the "override mistake". That is ordinary
  // expression code: `counts[w]` with `w` of "constructor" or "valueOf" uses an object as a
  // dictionary, and `a.map = …` gives one array its own version of a method. So every name that
  // was shadowable before the freeze becomes an accessor whose setter writes an own property on
  // the instance, which leaves the shadowing that worked before this block working after it.
  var tolerate = function (p, n) {
    var d = describe(p, n);
    // An accessor has no value to stand in for, and a name already non-writable or
    // non-configurable could not be shadowed before this ran either: no regression to repair.
    if (!d || !d.configurable || !('value' in d) || !d.writable) return;
    var v = d.value;
    define(p, n, {
      get: function () { return v; },
      set: function (nv) {
        // This setter stands in for the data property it replaced, so it answers a write the way
        // that property would: an accessor is handed the receiver and nothing else, while a data
        // property has the receiver's own property consulted first
        // (OrdinarySetWithOwnDescriptor). Doing that here keeps the repair invisible — an own
        // writable property is updated in place and keeps its other attributes, an own
        // non-writable one drops the write rather than being overwritten, and a target that
        // cannot take a new property is left alone instead of being handed to `defineProperty`,
        // which would throw. That last case covers a frozen prototype, an object the expression
        // froze itself, and `null` or `undefined`.
        if (this === p || this === null || this === undefined) return;
        var own = describe(this, n);
        if (own) {
          if (own.writable) define(this, n, { value: nv });
          return;
        }
        if (!isExtensible(this)) return;
        define(this, n, { value: nv, writable: true, enumerable: true, configurable: true });
      },
      enumerable: d.enumerable,
      configurable: false,
    });
  };

  var freezeProto = function (p) {
    if (p === null || (typeof p !== 'object' && typeof p !== 'function') || isFrozen(p)) return;
    var names = getNames(p);
    for (var i = 0; i < names.length; i++) if (!readByHost(p, names[i])) tolerate(p, names[i]);
    // Symbol keys go the same way: `a[Symbol.iterator] = …` on an expression's own array is an
    // ordinary way to make something iterable, and left frozen it is dropped in silence while
    // `for…of` quietly walks the array's own elements instead.
    var syms = getSyms(p);
    for (var j = 0; j < syms.length; j++) tolerate(p, syms[j]);
    freeze(p);
  };

  // Freezing a function also makes its own `prototype` property non-writable, so `Comp.prototype =
  // {}` is a no-op; freezing that prototype object closes the methods hanging off it.
  var shut = function (o) {
    var d = describe(o, 'prototype');
    if (d && 'value' in d) freezeProto(d.value);
    freeze(o);
  };

  var globals = getNames(g);
  for (var k = 0; k < globals.length; k++) {
    var n = globals[k];
    var d = describe(g, n);
    if (!d || !('value' in d) || typeof d.value !== 'function') continue;
    // The function object as well as the binding: the prelude calls `Array.isArray` and
    // `Object.setPrototypeOf` while it answers an expression, so a replacement for one of those
    // would not merely shadow a name, it would change what a later expression computes.
    shut(d.value);
    lock(n);
  }
  // `Math.sin = …` goes the same way, and the prelude leans on both of these itself. They are the
  // `prototype` of no function, so the loop above never reaches them, and they are the prototype
  // of no object either: nothing inherits `sin` or `parse`, so there is no shadowing for the
  // accessor repair to keep, and a plain freeze closes them. That matters on the hot path, where
  // a repaired name costs an accessor call on every read and a per-frame expression reads
  // `Math.*` more than anything else. The bindings are locked as well, so `Math = {}` cannot take
  // them away from the next expression either.
  freeze(Math);
  freeze(JSON);
  lock('Math');
  lock('JSON');
  // The iterator intrinsics are the `prototype` of no global function either, and `for…of`,
  // destructuring and spread all run through them: replacing `next` on the array iterator left
  // every later expression on the thread unable to iterate at all.
  var walk = function (o) {
    for (var p = o; p !== null && p !== undefined; p = protoOf(p)) freezeProto(p);
  };
  var probes = [function () { return [][Symbol.iterator](); },
    function () { return ''[Symbol.iterator](); },
    function () { return new Map()[Symbol.iterator](); },
    function () { return new Set()[Symbol.iterator](); },
    function () { return (function* () {})(); },
    function () { return 'a'.matchAll(/a/g); },
    function () { return Int8Array.prototype; }];
  for (var m = 0; m < probes.length; m++) {
    // Each one is a probe for an intrinsic this engine may not have at all.
    try { walk(protoOf(probes[m]())); } catch (e) { /* not present */ }
  }
})();
