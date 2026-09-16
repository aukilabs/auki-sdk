import test from 'node:test';
import assert from 'node:assert/strict';
import { ScreenHistory, isScreen, focusScreenTarget } from '../src/screens.ts';

test('dedicated screens return through preview and record to the original primary screen', () => {
  const navigation = new ScreenHistory();
  navigation.reset('data');
  navigation.go('record'); navigation.go('preview'); navigation.go('technical');
  assert.equal(navigation.back(), 'preview');
  assert.equal(navigation.back(), 'record');
  assert.equal(navigation.back(), 'data');
});
test('primary navigation clears obsolete back paths and logout clears all routes', () => {
  const navigation = new ScreenHistory();
  navigation.reset('data'); navigation.go('record'); navigation.go('networking'); navigation.go('settings');
  assert.equal(navigation.back(), 'networking');
  navigation.reset('access'); navigation.go('settings');
  assert.equal(navigation.back(), 'access');
});
test('screen routing rejects arbitrary targets and repeated routes do not trap Back', () => {
  assert.equal(isScreen('upload'), true); assert.equal(isScreen('not-a-screen'), false);
  const navigation = new ScreenHistory();
  navigation.reset('portals'); navigation.go('technical'); navigation.go('technical');
  assert.equal(navigation.back(), 'portals');
});


test('Back preserves native and explicit control tabIndex; only headings become programmatic targets', () => {
  for (const [tagName, tabIndex, expected] of [['BUTTON', 0, 0], ['INPUT', 2, 2], ['H1', 0, -1]] as const) {
    let focused = false;
    const target = { tagName, tabIndex: Number(tabIndex), focus(options: FocusOptions) { assert.equal(options.preventScroll, true); focused = true; } };
    focusScreenTarget(target);
    assert.equal(target.tabIndex, expected); assert.equal(focused, true);
  }
});
