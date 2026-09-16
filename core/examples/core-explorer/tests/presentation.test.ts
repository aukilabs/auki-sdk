import test from 'node:test';
import assert from 'node:assert/strict';
import { facts, technical, shortPeerId } from '../src/presentation.ts';

// Minimal DOM sink: assigning HTML is forbidden so these tests exercise text-only rendering.
class TextSink {
  tag: string;
  className = '';
  textContent = '';
  children: TextSink[] = [];
  constructor(tag: string) { this.tag = tag; }
  set innerHTML(_: string) { throw new Error('Provider content must never use HTML'); }
  append(...children: TextSink[]) { this.children.push(...children); }
}
const flatten = (node: TextSink): string => node.textContent + node.children.map(flatten).join(' ');

test('readable facts keep hostile text inert, redact credentials and omit unavailable values', () => {
  const original = Object.getOwnPropertyDescriptor(globalThis, 'document');
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { createElement: (tag: string) => new TextSink(tag) } });
  try {
    const node = facts({ Name: '<img src=x onerror=alert(1)>', Notes: 'password=fixture-only', Position: { x: 0, token: 'fixture-secret' }, Absent: null }) as unknown as TextSink;
    assert.equal(node.children.length, 3);
    const rendered = flatten(node);
    assert.match(rendered, /<img src=x onerror=alert\(1\)>/);
    assert.match(rendered, /\[redacted\]/);
    assert.doesNotMatch(rendered, /fixture-only|fixture-secret|Absent/);
    assert.match(rendered, /"x": 0/);
    const raw = technical({ name: '<script>fixture</script>', secret: 'fixture-secret' }) as unknown as TextSink;
    assert.equal(raw.tag, 'details');
    assert.match(flatten(raw), /<script>fixture<\/script>/);
    assert.doesNotMatch(flatten(raw), /fixture-secret/);
  } finally {
    if (original) Object.defineProperty(globalThis, 'document', original);
    else Reflect.deleteProperty(globalThis, 'document');
  }
});

test('short peer identifiers retain actual ends and redact before shortening', () => {
  const peer = '12D3KooW' + 'a'.repeat(40) + 'realTail';
  assert.equal(shortPeerId(peer), `${peer.slice(0, 12)}…realTail`);
  assert.equal(shortPeerId('short-id'), 'short-id');
  assert.equal(shortPeerId('Bearer SYNTHETIC_PRIVATE_CREDENTIAL'), 'Bearer [redacted]');
});
