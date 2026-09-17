import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { UploadController } from '../src/upload.ts';

function handler(file: string, prefix: string) {
  const tree = ts.createSourceFile(file, readFileSync(new URL(`../src/${file}`, import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true);
  let source = '';
  function visit(node: ts.Node) {
    if (ts.isExpressionStatement(node) && node.getText(tree).startsWith(prefix)) source = node.getText(tree);
    ts.forEachChild(node, visit);
  }
  visit(tree); assert.ok(source, prefix);
  return ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
}
const keyboard = handler('main.ts', "document.addEventListener('keydown'");
const uploadBack = handler('upload-ui.ts', "button('upload-back').onclick");
const tick = () => new Promise(resolve => setImmediate(resolve));
for (const method of ['Escape', 'Back']) for (const sending of [false, true]) {
  test(`actual ${method} upload handler invalidates review / aborts held send: sending=${sending}`, async () => {
    let signal: AbortSignal | undefined, release!: () => void, key!: (event: unknown) => void, writes = 0;
    const held = new Promise<void>(resolve => { release = resolve; });
    const binding = { domainId: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', domainName: 'Offline', environment: 'fixture', data: {
      write: async (_target: unknown, _bytes: unknown, abort?: AbortSignal) => { writes++; signal = abort; await held; throw Error('offline end'); },
      get: async () => { throw Error('unused'); },
    } };
    const controller = new UploadController(() => binding, () => {}, () => {}, () => 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb');
    const navigation = { current: 'upload' };
    const back: any = { disabled: false, closest: () => null, click() { this.onclick(); } };
    runInNewContext(keyboard + uploadBack, {
      controller, navigation, button: () => back, options: { navigate: (route: string) => { navigation.current = route; } },
      document: { addEventListener: (_event: string, fn: typeof key) => { key = fn; }, querySelector: (selector: string) => selector.includes('#upload-back') ? back : null },
      backView: () => { navigation.current = 'data'; },
    });
    controller.review({ name: 'fixture.txt', size: 1, arrayBuffer: async () => new Uint8Array([65]).buffer }, 'example.text');
    const target = controller.state.review!.target;
    const pending = sending ? controller.confirm() : Promise.resolve();
    await tick();
    if (method === 'Escape') key({ key: 'Escape', defaultPrevented: false }); else back.click();
    assert.equal(navigation.current, 'data');
    if (sending) {
      assert.equal(signal?.aborted, true);
      assert.equal(controller.state.outcome, 'uncertain');
      assert.equal(controller.state.review?.target, target, 'retain exact target for reconciliation');
    } else {
      assert.notEqual(controller.state.step, 'review');
      await controller.confirm(); assert.equal(writes, 0, 'old review cannot send');
    }
    release(); await pending;
    if (sending) assert.equal(controller.state.outcome, 'uncertain', 'late settlement cannot erase uncertainty');
  });
}
