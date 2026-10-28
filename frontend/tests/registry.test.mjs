import assert from 'node:assert/strict';
import test from 'node:test';
import { describeQueryError, formatTimestamp, unwrapResult, withQueryTimeout } from '../src/lib/registry.ts';

test('nanoseconds and milliseconds produce the same date without losing bigint precision', () => {
  const milliseconds = BigInt(Date.parse('2026-10-27T18:00:00.123Z'));
  assert.equal(formatTimestamp(milliseconds * 1_000_000n + 999_999n, 'ns').iso, '2026-10-27T18:00:00.123Z');
  assert.equal(formatTimestamp(milliseconds, 'ms').iso, '2026-10-27T18:00:00.123Z');
});

test('out-of-range timestamps remain renderable', () => {
  assert.equal(formatTimestamp(18_446_744_073_709_551_615n, 'ms').iso, undefined);
});

test('an empty Ok result is data, not an error', () => {
  assert.deepEqual(unwrapResult({ __kind__: 'Ok', Ok: [] }), []);
});

test('registry errors retain the entire variant payload', () => {
  assert.throws(() => unwrapResult({ __kind__: 'Err', Err: { __kind__: 'InvalidInput', InvalidInput: 'cursor inválido' } }), /cursor inválido/);
});

test('a real-shaped expiry rejection stops queries and preserves its full details', () => {
  const error = new Error('Invalid request expiry: Provided ingress_expiry is outside the expected range');
  const failure = describeQueryError(error);
  assert.equal(failure.clockError, true);
  assert.equal(failure.detail, error.stack);
});

test('network and configuration failures have different actionable messages', () => {
  assert.match(describeQueryError(new TypeError('Failed to fetch')).title, /contactar/);
  assert.match(describeQueryError(new Error('Missing cookie ic_env')).title, /identificar/);
});

test('nested causes retain the clock rejection instead of hiding it behind a network error', () => {
  const failure = describeQueryError(new Error('Failed to fetch', { cause: new Error('Invalid request expiry: ingress_expiry') }));
  assert.equal(failure.clockError, true);
  assert.match(failure.detail, /Failed to fetch/);
  assert.match(failure.detail, /Invalid request expiry/);
});

test('queries that never settle time out', async () => {
  await assert.rejects(withQueryTimeout(new Promise(() => {}), 10), /no respondió/);
});

test('a rejection is preserved and a successful response is returned', async () => {
  const error = new Error('backend unavailable');
  await assert.rejects(withQueryTimeout(Promise.reject(error)), (received) => received === error);
  assert.deepEqual(await withQueryTimeout(Promise.resolve([])), []);
});
