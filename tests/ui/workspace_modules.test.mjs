import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const directory = new URL('../../assets/admin/', import.meta.url);
const source = name => readFile(new URL(name, directory), 'utf8');
const dataURL = text => `data:text/javascript;base64,${Buffer.from(text).toString('base64')}`;
const urls = {};
for (const name of ['icons', 'api', 'ui', 'log-settings', 'key-manager', 'workspace', 'backend-logs', 'model-testing', 'playground', 'virtual-models']) {
  let text = await source(`${name}.js`);
  for (const [dependency, url] of Object.entries(urls)) text = text.replaceAll(`'./${dependency}.js'`, JSON.stringify(url));
  urls[name] = dataURL(text);
}
const keys = await import(urls['key-manager']);
const backend = await import(urls['backend-logs']);
const workspace = await import(urls.workspace);
const testing = await import(urls['model-testing']);
const playground = await import(urls.playground);
const virtualModels = await import(urls['virtual-models']);
const api = await import(urls.api);

await test('SSE frames handle LF, CRLF, multiple frames and partial tails', () => {
  assert.deepEqual(api.parseSseFrames('data: one\r\n\r\ndata: two\n\ndata: tail'), { frames: ['data: one','data: two'],rest:'data: tail' });
});
await test('playground decodes Chat and Responses reasoning, usage and terminal events', () => {
  assert.equal(playground.playgroundDelta({ choices:[{delta:{content:'hello',reasoning_content:'think'}}] }).thinking,'think');
  assert.equal(playground.playgroundDelta({ type:'response.output_text.delta', delta:'hello' }).text,'hello');
  assert.deepEqual(playground.playgroundDelta({ type:'response.completed', response:{usage:{input_tokens:0}} }),{ usage:{input_tokens:0},terminal:true });
  assert.throws(() => playground.playgroundDelta({ type:'response.failed', response:{error:{message:'bad'}} }), /bad/);
});
await test('stream client keeps UTF-8 boundaries and CSRF without exposing saved API keys', async () => {
  const previous = globalThis.fetch;
  const bytes = new TextEncoder().encode('data: {"choices":[{"delta":{"content":"你好"}}]}\r\n\r\ndata: {"choices":[{"delta":{},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n');
  globalThis.fetch = async (url, options) => {
    assert.equal(url,'/admin/api/test/stream'); assert.equal(options.headers['X-CSRF-Token'],'csrf'); assert.equal(Object.hasOwn(options.headers,'Authorization'),false);
    return new Response(new ReadableStream({ start(controller) { for (const byte of bytes) controller.enqueue(Uint8Array.of(byte)); controller.close(); } }),{ headers:{'x-request-id':'request'} });
  };
  try {
    api.setSession({csrf_token:'csrf'}); const events=[];
    const metadata = await api.stream('/test/stream',{position:1},{onEvent:event => events.push(event)});
    assert.equal(events[0].choices[0].delta.content,'你好'); assert.equal(events.length,2); assert.equal(metadata.requestId,'request');
  } finally { globalThis.fetch=previous; api.setSession(null); }
});
await test('virtual model forms retain target order, nested aliases and status conditions', () => {
  assert.deepEqual(virtualModels.virtualModelDocument([{name:'assistant',targets:[{provider:'primary',model:'real',statuses:'',source:{metadata:1}},{provider:'@virtual',model:'fallback',statuses:''},{provider:'backup',model:'other',statuses:'429, 502'}]}]),{assistant:[{metadata:1,provider:'primary',model:'real'},{virtual:'fallback'},{provider:'backup',model:'other',on_status:[429,502]}]});
  assert.throws(() => virtualModels.virtualModelDocument([{name:'x',targets:[]}]));
  assert.throws(() => virtualModels.virtualModelDocument([{name:'x',targets:[{provider:'p',model:'m',statuses:'400.5'}]}]));
});

await test('virtual model forms serialize prototype-like aliases as ordinary own properties', () => {
  const targets = [{provider:'primary',model:'real',statuses:''}];
  const definitions = virtualModels.virtualModelDocument(['__proto__','constructor','toString'].map(name => ({name,targets})));
  assert.equal(Object.getPrototypeOf(definitions), Object.prototype);
  const saved = JSON.parse(JSON.stringify(definitions));
  for (const name of ['__proto__','constructor','toString']) {
    assert.equal(Object.hasOwn(saved,name),true);
    assert.deepEqual(saved[name],[{provider:'primary',model:'real'}]);
  }
  assert.throws(() => virtualModels.virtualModelDocument([{name:'__proto__',targets},{name:'__proto__',targets}]));
});

await test('group names are trimmed and deduplicated without losing Unicode or case', () => {
  assert.deepEqual(keys.parseGroups('prod, 开发，prod\nDEV'), ['prod', '开发', 'DEV']);
  assert.deepEqual(keys.parseGroups(' , \n'), []);
});
await test('key filters combine names, groups, model rules and enabled status', () => {
  const records = [
    { id: 'one', name: 'Phone', groups: ['dev'], enabled: true, models: ['sample'] },
    { id: 'two', name: 'Server', groups: ['prod', 'dev'], enabled: false, models: [{ 'channel/sample': 2 }] },
  ];
  assert.deepEqual(keys.filterKeys(records, { group: 'dev', status: 'disabled', query: 'CHANNEL' }).map(key => key.id), ['two']);
  assert.deepEqual(keys.filterKeys(records, { group: 'prod', status: 'enabled' }), []);
  assert.deepEqual(keys.keyModelRules(records[1]), ['channel/sample']);
});
await test('group model loading excludes other groups and keeps shared models once', () => {
  const channels = [{ groups: ['prod'], models: ['private'] }, { groups: ['dev'], models: ['shared', 'dev'] }, { groups: ['dev', 'prod'], models: ['shared'] }];
  assert.deepEqual(keys.groupModels(channels, ['dev']), ['dev', 'shared']);
  assert.deepEqual(keys.groupModels(channels, ['missing']), []);
});
await test('incremental logs are ordered, deduplicated and bounded', () => {
  const previous = [{ id: 1 }, { id: 2, message: 'old' }];
  assert.deepEqual(backend.mergeLogEntries(previous, [{ id: 3 }, { id: 2, message: 'new' }], 2), [{ id: 2, message: 'new' }, { id: 3 }]);
  assert.equal(previous[1].message, 'old');
  const query = backend.backendLogQuery(42, 'ERROR', 'a & b');
  assert.equal(query.get('after'), '42');
  assert.equal(query.get('q'), 'a & b');
  assert.equal(query.get('limit'), '500');
});
await test('workspace parent navigation stays at the virtual root', () => {
  assert.equal(workspace.parentPath(''), '');
  assert.equal(workspace.parentPath('docs'), '');
  assert.equal(workspace.parentPath('docs/adr'), 'docs');
});
await test('test requests use the selected model and no invented token budget', () => {
  assert.deepEqual(testing.testPayload('/v1/chat/completions', 'selected', 'hello', '', { model: 'ignored', stream: true, temperature: 0 }), {
    model: 'selected', stream: false, temperature: 0, messages: [{ role: 'user', content: 'hello' }],
  });
  assert.deepEqual(testing.testPayload('/v1/responses', 'selected', 'hello', '128'), { model: 'selected', stream: false, input: 'hello', max_output_tokens: 128 });
});
await test('an unavailable target never silently becomes a different channel test', () => {
  const rows = [{ provider: 'one', model: 'sample' }, { provider: 'two', model: 'sample' }, { provider: 'two', model: 'other' }];
  assert.deepEqual(testing.availableTestModels(rows, 'one', false), []);
  assert.deepEqual(testing.availableTestModels(rows, 'missing', true), []);
  assert.deepEqual(testing.availableTestModels(rows, 'one', true), ['sample']);
  assert.deepEqual(testing.availableTestModels(rows), ['sample', 'other']);
});
await test('test usage distinguishes zero from missing values for both protocols', () => {
  assert.deepEqual(testing.testUsage({}), { input: undefined, output: undefined, cached: undefined });
  assert.deepEqual(testing.testUsage({ usage: { prompt_tokens: 10, completion_tokens: 0, prompt_tokens_details: { cached_tokens: 0 } } }), { input: 10, output: 0, cached: 0 });
  assert.deepEqual(testing.testUsage({ usage: { input_tokens: 12, output_tokens: 3, input_tokens_details: { cached_tokens: 8 } } }), { input: 12, output: 3, cached: 8 });
});
await test('test output renders text from Chat and Responses without reasoning noise', () => {
  assert.equal(testing.testOutput({ choices: [{ message: { content: 'chat answer' } }] }), 'chat answer');
  assert.equal(testing.testOutput({ output: [{ type: 'reasoning' }, { content: [{ type: 'output_text', text: 'answer' }] }] }), 'answer');
});
await test('live logs stop polling when hidden or unmounted and testing never starts on mount', async () => {
  const logs = await source('backend-logs.js');
  assert.match(logs, /!document.hidden/);
  assert.match(logs, /signal.addEventListener\('abort'/);
  assert.match(logs, /clearTimeout\(timer\)/);
  const tests = await source('model-testing.js');
  assert.match(tests, /await loadModels\(\);/);
  assert.doesNotMatch(tests, /await run\(/);
  for (const name of ['key-manager.js', 'workspace.js', 'model-testing.js']) assert.doesNotMatch(await source(name), /(?:localStorage|sessionStorage)\./);
});
