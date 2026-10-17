import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';
import { setImmediate as nextTurn } from 'node:timers/promises';

const root = new URL('../../assets/admin/', import.meta.url);
const source = name => readFile(new URL(name, root), 'utf8');
const dataURL = value => `data:text/javascript;base64,${Buffer.from(value).toString('base64')}`;
const iconsURL = dataURL(await source('icons.js'));
const icons = await import(iconsURL);
const uiURL = dataURL((await source('ui.js')).replace("'./icons.js'", JSON.stringify(iconsURL)));
const ui = await import(uiURL);
const apiURL = dataURL(await source('api.js'));
const balanceSource = await source('channel-balances.js');
const balances = await import(dataURL(balanceSource.replace("'./icons.js'", JSON.stringify(iconsURL)).replace("'./ui.js'", JSON.stringify(uiURL)).replace("'./api.js'", JSON.stringify(apiURL))));
const saved = () => ({ provider: 'codex', status: 'complete', keys: [{ position: 1, status: 'ok', plan: 'plus', checked_at: 100, windows: [{ label: '短期额度', limit_seconds: 18000, remaining_percent: 75, reset_at: 1900000000 }, { label: '长期额度', limit_seconds: 604800, remaining_percent: 0 }] }] });

await test('balances show Codex plan and separate remaining windows, including exhausted zero', () => {
  const html = balances.balanceDetails(saved());
  assert.match(html, /Plus/);
  assert.match(html, /5 小时/);
  assert.match(html, /7 天/);
  assert.match(html, /剩余 75%/);
  assert.match(html, /剩余 0%/);
  assert.match(html, /重置时间/);
  assert.ok(!html.includes('USD'));
  assert.match(balances.balanceSummary(saved()), /Plus/);
});

await test('balances keep NovelAI Anlas, component balances and expiry separate from currency', () => {
  const snapshot = { keys: [{ position: 2, status: 'ok', plan: 'Opus', active: false, amount: 1000, currency: 'Anlas', expires_at: 1900000000, checked_at: 100, buckets: [{ label: '订阅 Anlas', amount: 900, unit: 'Anlas' }, { label: '购买 Anlas', amount: 100, unit: 'Anlas' }] }] };
  const html = balances.balanceDetails(snapshot);
  assert.match(html, /1,000 Anlas/);
  assert.match(html, /订阅 Anlas/);
  assert.match(html, /购买 Anlas/);
  assert.match(html, /套餐到期/);
  assert.match(html, /套餐未生效/);
  assert.ok(!html.includes('USD'));
  assert.ok(!html.includes('100%'));
});

await test('the shared panel displays a new plugin plan and quota units without supplier-specific UI', () => {
  const snapshot = { keys: [{ position: 1, status: 'ok', plan: 'Studio Custom', amount: 42, currency: '积分',
    windows: [{ label: '每月任务', remaining: 40, unit: '次' }],
    buckets: [{ label: '额外资源', amount: 3, unit: 'GB' }] }] };
  const html = balances.balanceDetails(snapshot);
  assert.match(html, /Studio Custom/);
  assert.match(html, /42 积分/);
  assert.match(html, /每月任务/);
  assert.match(html, /40 次/);
  assert.match(html, /额外资源/);
  assert.match(html, /3 GB/);
  assert.ok(!html.includes('USD'));
});

await test('balances do not turn unknown values, missing allowances or unsupported providers into zero', () => {
  assert.match(balances.balanceSummary(null), /未获取/);
  assert.match(balances.balanceDetails({ status: 'unsupported', keys: [] }), /渠道未提供余额查询能力/);
  const html = balances.balanceDetails({ keys: [{ position: 1, status: 'ok', plan: 'Pro', amount: null, checked_at: 100, windows: [{ label: '额度', remaining_percent: null }] }] });
  assert.match(html, /剩余额度未知/);
  assert.ok(!html.includes('剩余 0%'));
});

await test('balances escape upstream display fields and mark old data after failure or configuration changes', () => {
  const snapshot = saved();
  snapshot.configuration_changed = true;
  Object.assign(snapshot.keys[0], { plan: '<img src=x>', account_id: '<script>', last_error: 'timeout', last_attempt_at: 200 });
  const html = balances.balanceDetails(snapshot);
  assert.ok(!html.includes('<img src=x>'));
  assert.ok(!html.includes('<script>'));
  assert.match(html, /&lt;img src=x&gt;/);
  assert.match(html, /配置已更改/);
  assert.match(html, /保留上次成功结果/);
  assert.match(html, /剩余 75%/);
});

await test('wallet availability is not described as subscription expiry and ISO resets remain visible', () => {
  const html = balances.balanceDetails({ keys: [{ position: 1, status: 'ok', amount: 0, currency: 'CNY', available: false,
    windows: [{ label: '套餐限额', remaining_percent: 75, reset_at: '2026-10-18T00:00:00Z' }] }] });
  assert.match(html, /余额不可用/);
  assert.ok(!html.includes('套餐未生效'));
  assert.match(html, /重置时间/);
  assert.ok(!html.includes('Invalid Date'));
  assert.match(balances.balanceDetails({ status: 'plugin_unavailable', keys: [] }), /余额插件未安装/);
});

function panelFixture(request) {
  const button = { disabled: false, listeners: {}, addEventListener(name, listener) { this.listeners[name] = listener; } };
  const output = { innerHTML: '' }, errors = { hidden: true, innerHTML: '' };
  const container = { innerHTML: '', querySelector(selector) { return { '[data-balance-refresh]': button, '[data-balance-content]': output, '[data-balance-error]': errors }[selector]; } };
  const factory = runInNewContext(balanceSource.replace(/^import .*;\r?\n/gm, '').replace(/^export /gm, '') + '\nmountChannelBalance', {
    ...ui, icon: icons.icon, AbortController, encodeURIComponent, api: { request },
  });
  return { mount: options => factory(container, '渠道 A', options), button, output, errors };
}

await test('balances never poll: an initial snapshot sends nothing until the refresh button is clicked', async () => {
  const requests = [], updates = [];
  const fixture = panelFixture(async (path, options) => { requests.push([path, options]); return saved(); });
  fixture.mount({ initial: saved(), onUpdated: snapshot => updates.push(snapshot) });
  assert.equal(requests.length, 0);
  await fixture.button.listeners.click();
  assert.equal(requests.length, 1);
  assert.equal(requests[0][0], '/channels/balances');
  assert.equal(requests[0][1].method, 'POST');
  assert.equal(requests[0][1].body.provider, '渠道 A');
  assert.equal(requests[0][1].timeoutMs, 60000);
  assert.equal(updates.length, 1);
  assert.equal(fixture.button.disabled, false);
  assert.match(fixture.output.innerHTML, /Plus/);
});

await test('opening the channel editor only reads persisted balances, not upstream quotas', async () => {
  const requests = [];
  const fixture = panelFixture(async (path, options) => { requests.push([path, options]); return { balances: { '渠道 A': saved() } }; });
  fixture.mount();
  await nextTurn();
  assert.equal(requests.length, 1);
  assert.equal(requests[0][1].method, 'GET');
  assert.equal(requests[0][0], '/channels/balances?provider=%E6%B8%A0%E9%81%93%20A');
  assert.match(fixture.output.innerHTML, /剩余 75%/);
});

await test('failed balance refresh keeps the already rendered values and re-enables the button', async () => {
  let fail = false;
  const fixture = panelFixture(async () => { if (fail) throw new Error('disk full'); return saved(); });
  const panel = fixture.mount({ initial: saved() });
  await panel.refresh();
  const previous = fixture.output.innerHTML;
  fail = true;
  await panel.refresh();
  assert.equal(fixture.output.innerHTML, previous);
  assert.match(fixture.errors.innerHTML, /disk full/);
  assert.equal(fixture.errors.hidden, false);
  assert.equal(fixture.button.disabled, false);
});

await test('closing balance panel cancels the request and ignores its late result', async () => {
  let finish, signal;
  const fixture = panelFixture((path, options) => { signal = options.signal; return new Promise(resolve => { finish = resolve; }); });
  const panel = fixture.mount({ initial: saved() });
  const pending = panel.refresh();
  panel.destroy();
  assert.equal(signal.aborted, true);
  finish(saved());
  await pending;
  assert.equal(fixture.output.innerHTML, '');
});

await test('channel rows expose the real balance button and saved summary even in read-only configuration', async () => {
  const text = await source('app.js');
  const declarations = text.split('\n').filter(line => line.startsWith('const mini =') || line.startsWith('const tags =')).join('\n');
  const rows = runInNewContext(declarations + '\n' + text.slice(text.indexOf('function channelRows('), text.indexOf('async function channels(')) + '\nchannelRows', {
    ...ui, icon: icons.icon, aiIcon: icons.aiIcon, channelBrand: icons.channelBrand, balanceSummary: balances.balanceSummary,
    state: { balances: { codex: saved() } },
  });
  const html = rows([{ provider: 'codex', base_url: 'https://chatgpt.com', engine: 'codex', models: [], api_key_count: 1 }], false);
  assert.match(html, /data-action="channel-balance"/);
  assert.match(html, /data-channel-balance-index="0"/);
  assert.match(html, /剩余 75%/);
  assert.ok(!html.includes('data-action="delete-channel"'));
});

await test('saved balance read failure is visible without hiding the channel list', async () => {
  const text = await source('app.js');
  const render = runInNewContext(text.slice(text.indexOf('async function channels('), text.indexOf('async function editChannel(')) + '\nchannels', {
    ...ui, icon: icons.icon, read: async () => ({ channels: [{ provider: 'codex', models: [] }], writable: true }),
    api: { get: async () => { throw new Error('saved balance file unreadable'); } }, state: {}, URLSearchParams,
    window: { location: { hash: '#channels' } }, head: () => '', button: () => '', table: () => '<table>channels remain</table>', channelRows: () => '',
  });
  const html = await render();
  assert.match(html, /saved balance file unreadable/);
  assert.match(html, /channels remain/);
});
