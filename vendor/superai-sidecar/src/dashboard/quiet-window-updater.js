/**
 * v2.0.67 — Quiet-window auto-update detector (#112).
 *
 * Watches per-minute request rate via a tiny timestamp ring and, when the
 * proxy stays under a configurable threshold for the full window length,
 * triggers the existing docker self-update flow. The point is to land a
 * new image during a real lull rather than mid-burst — operators who run
 * the proxy 24/7 want updates applied automatically but not in the middle
 * of a tool-calling agent loop.
 *
 * Design choices and trade-offs
 * ──────────────────────────────
 * Ring buffer holds raw `Date.now()` timestamps for every observed request.
 * On each tick we drop entries older than the window and count the rest.
 * The buffer is bounded — the threshold check only needs ≤ threshold + 1
 * entries to decide quietness, so we cap retention at a generous 4 × the
 * window's worth of expected traffic to keep memory predictable even on
 * busy boxes that never trigger an update.
 *
 * No actual scheduling here — the caller (src/index.js boot) calls
 * `startQuietWindowAutoUpdate()` once on startup, which arms a setInterval
 * that polls every `tickIntervalMs` (default 60s). Polling is cheap
 * enough that we don't bother with a real scheduler library.
 *
 * Cooldown after a successful update prevents flapping if the operator
 * ships several releases in a row — a 24h default means even if traffic
 * goes quiet for hours after the first update, we won't pull again for a
 * day. Cooldown is in-memory; on container recreate the new container's
 * lastUpdateAt resets to 0 (which is fine — the new container IS the
 * update we just applied).
 *
 * Cold-start grace ensures we don't immediately auto-update right after
 * boot when the request ring is empty by definition. Without this, every
 * fresh container would self-update once before any real traffic arrived.
 *
 * Disabled by default. Operator opts in via runtime-config
 * `experimental.autoUpdateQuietWindow` flag (dashboard toggle exists in
 * the experimental panel; persisted across restarts).
 */

import { log } from '../config.js';
import {
  getRuntimeConfig, isExperimentalEnabled, setExperimental,
} from '../runtime-config.js';
import { detectDockerSelfUpdate, runDockerSelfUpdate } from './docker-self-update.js';

// ── Tunables (overridable from runtime-config + env) ────────────────

const DEFAULTS = {
  windowMinutes: 5,        // length of the lull window
  thresholdRequests: 5,    // request count above which we're "busy"
  cooldownHours: 24,       // suppress further updates for this long after success
  tickIntervalMs: 60_000,  // how often the watcher checks (1 min)
  coldStartGraceMs: 10 * 60 * 1000, // wait 10 min after boot before first eligible tick
};

const _state = {
  ring: [],                // [ts, ts, ts, ...] of recent request times (Date.now)
  ringCap: 0,              // recomputed when window/threshold change
  startedAt: Date.now(),
  lastUpdateAt: 0,
  lastTickAt: 0,
  lastResult: null,        // { ok, reason, image, ... } from runDockerSelfUpdate
  timer: null,
  // For tests: lets us inject a fake clock + run-update fn so we can step
  // the watcher without real timers / docker.
  _now: () => Date.now(),
  _runUpdate: null,        // null → use real runDockerSelfUpdate; fn → use that
};

function effectiveSettings() {
  const cfg = getRuntimeConfig();
  const knobs = cfg?.autoUpdateQuietWindow || {};
  return {
    enabled: isExperimentalEnabled('autoUpdateQuietWindow'),
    windowMinutes: positiveInt(knobs.windowMinutes, DEFAULTS.windowMinutes),
    thresholdRequests: nonNegativeInt(knobs.thresholdRequests, DEFAULTS.thresholdRequests),
    cooldownHours: positiveInt(knobs.cooldownHours, DEFAULTS.cooldownHours),
    tickIntervalMs: positiveInt(knobs.tickIntervalMs, DEFAULTS.tickIntervalMs),
    coldStartGraceMs: positiveInt(knobs.coldStartGraceMs, DEFAULTS.coldStartGraceMs),
  };
}

function positiveInt(v, fallback) {
  const n = typeof v === 'number' ? v : Number(v);
  return Number.isFinite(n) && n > 0 ? Math.floor(n) : fallback;
}

function nonNegativeInt(v, fallback) {
  const n = typeof v === 'number' ? v : Number(v);
  return Number.isFinite(n) && n >= 0 ? Math.floor(n) : fallback;
}

// ── Request tracking (called from chat.js / messages.js / responses.js) ─

/**
 * Mark one observed request. Cheap — appends a timestamp to the ring and
 * trims at a generous cap. Safe to call without checking whether the
 * watcher is enabled; if it's off the ring just stays a memory cushion.
 */
export function markRequest(timestamp) {
  const ts = Number.isFinite(timestamp) ? timestamp : _state._now();
  _state.ring.push(ts);
  // Bound ring size at 4 × (threshold + 1) so we keep enough history to
  // judge any reasonable threshold without unbounded growth on busy hosts.
  // Recomputed here rather than at config time — the threshold change is
  // rare and this is cheap.
  const s = effectiveSettings();
  const cap = Math.max(64, (s.thresholdRequests + 1) * 4);
  _state.ringCap = cap;
  if (_state.ring.length > cap * 2) {
    // Drop oldest half in one slice to keep the average cost amortised.
    _state.ring.splice(0, _state.ring.length - cap);
  }
}

function pruneRing(now, windowMs) {
  const cutoff = now - windowMs;
  // Most-common case: only the head is stale. Find first index >= cutoff.
  let i = 0;
  while (i < _state.ring.length && _state.ring[i] < cutoff) i++;
  if (i > 0) _state.ring.splice(0, i);
}

/**
 * Return true if the proxy has been quiet enough to safely auto-update.
 * Pure inspector — does not mutate the ring or trigger anything.
 */
export function isQuiet(nowOverride) {
  const s = effectiveSettings();
  const now = Number.isFinite(nowOverride) ? nowOverride : _state._now();
  const cutoff = now - s.windowMinutes * 60_000;
  let count = 0;
  for (let i = _state.ring.length - 1; i >= 0; i--) {
    if (_state.ring[i] >= cutoff) count++;
    else break; // ring is timestamp-ordered, can stop on first miss
  }
  return count <= s.thresholdRequests;
}

/**
 * Whole-tick decision logic. Pulled out as a pure function so tests can
 * exercise every branch without timers or docker.
 *
 *   reason → 'disabled' | 'cold-start' | 'cooldown' | 'busy' | 'eligible'
 */
export function evaluateTick(nowOverride) {
  const s = effectiveSettings();
  const now = Number.isFinite(nowOverride) ? nowOverride : _state._now();
  if (!s.enabled) return { run: false, reason: 'disabled' };
  if (now - _state.startedAt < s.coldStartGraceMs) {
    return { run: false, reason: 'cold-start', untilMs: _state.startedAt + s.coldStartGraceMs - now };
  }
  if (_state.lastUpdateAt && now - _state.lastUpdateAt < s.cooldownHours * 3_600_000) {
    return {
      run: false,
      reason: 'cooldown',
      untilMs: _state.lastUpdateAt + s.cooldownHours * 3_600_000 - now,
    };
  }
  pruneRing(now, s.windowMinutes * 60_000);
  const count = _state.ring.length;
  if (count > s.thresholdRequests) {
    return { run: false, reason: 'busy', count, threshold: s.thresholdRequests };
  }
  return { run: true, reason: 'eligible', count, threshold: s.thresholdRequests };
}

// ── Watcher loop ────────────────────────────────────────────────────

async function tick() {
  _state.lastTickAt = _state._now();
  const decision = evaluateTick();
  if (!decision.run) {
    // Only log on state transitions (busy ↔ eligible) so the ticker doesn't
    // spam the log every minute. Compare to last decision via a closure-
    // scoped variable on _state.
    if (decision.reason !== _state._lastReason) {
      _state._lastReason = decision.reason;
      if (decision.reason === 'busy' || decision.reason === 'eligible') {
        log.debug(`quiet-window: ${decision.reason} (count=${decision.count} threshold=${decision.threshold})`);
      }
    }
    return decision;
  }
  // Eligible — try to actually run the update.
  _state._lastReason = 'eligible';
  log.info(`quiet-window: lull detected (count=${decision.count} ≤ threshold=${decision.threshold}); attempting docker self-update`);
  let result;
  try {
    const runFn = _state._runUpdate || runDockerSelfUpdate;
    result = await runFn();
  } catch (e) {
    result = { ok: false, reason: 'exception', detail: e.message };
  }
  _state.lastResult = { ...result, at: _state._now() };
  if (result?.ok) {
    _state.lastUpdateAt = _state._now();
    log.info(`quiet-window: self-update kicked off — image=${result.image} project=${result.project}`);
  } else {
    log.warn(`quiet-window: self-update declined or failed — reason=${result?.reason} detail=${result?.detail || ''}`);
  }
  return { ...decision, result };
}

/**
 * Start the watcher. Idempotent — calling twice replaces the existing
 * timer. Safe to call when the feature is disabled (the per-tick check
 * short-circuits before doing anything).
 */
export function startQuietWindowAutoUpdate(opts = {}) {
  stopQuietWindowAutoUpdate();
  if (opts._now) _state._now = opts._now;
  if (opts._runUpdate) _state._runUpdate = opts._runUpdate;
  _state.startedAt = _state._now();
  const s = effectiveSettings();
  _state.timer = setInterval(() => { tick().catch(() => {}); }, s.tickIntervalMs);
  if (typeof _state.timer.unref === 'function') _state.timer.unref();
  log.info(`quiet-window: watcher started (window=${s.windowMinutes}min threshold=${s.thresholdRequests} cooldown=${s.cooldownHours}h enabled=${s.enabled})`);
  return _state.timer;
}

export function stopQuietWindowAutoUpdate() {
  if (_state.timer) {
    clearInterval(_state.timer);
    _state.timer = null;
  }
}

// ── Inspection helpers (used by dashboard API + tests) ──────────────

export function getStatus() {
  const s = effectiveSettings();
  const now = _state._now();
  const decision = evaluateTick(now);
  return {
    enabled: s.enabled,
    settings: {
      windowMinutes: s.windowMinutes,
      thresholdRequests: s.thresholdRequests,
      cooldownHours: s.cooldownHours,
      coldStartGraceMs: s.coldStartGraceMs,
    },
    startedAt: _state.startedAt,
    lastUpdateAt: _state.lastUpdateAt,
    lastTickAt: _state.lastTickAt,
    lastResult: _state.lastResult,
    ringSize: _state.ring.length,
    decision,
  };
}

export function setEnabled(on) {
  setExperimental({ autoUpdateQuietWindow: !!on });
  return getStatus();
}

// Test seam: reset internal state so unit tests can run independently.
// Not exported via the package surface; only used by the test file.
export function _resetForTest() {
  _state.ring = [];
  _state.startedAt = 0;
  _state.lastUpdateAt = 0;
  _state.lastTickAt = 0;
  _state.lastResult = null;
  _state._lastReason = null;
  _state._now = () => Date.now();
  _state._runUpdate = null;
  stopQuietWindowAutoUpdate();
}

// Test seam: inject mocks for clock + update runner.
export function _injectForTest({ now, runUpdate, startedAt } = {}) {
  if (typeof now === 'function') _state._now = now;
  if (typeof runUpdate === 'function') _state._runUpdate = runUpdate;
  if (Number.isFinite(startedAt)) _state.startedAt = startedAt;
}

// One-shot tick for tests + dashboard "trigger now" button.
export async function _runOneTick() {
  return tick();
}
