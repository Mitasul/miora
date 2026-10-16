// No npm dependencies. Run: node tests/ui/admin_modules.test.mjs
// API mocks are isolated test fixtures; production UI always calls the real backend.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';
const assets = new URL('../../assets/admin/', import.meta.url);
const source = name => readFile(new URL(name, assets), 'utf8');
const themeSource = await source('theme.js');
function themeFixture({ stored = null, dark = false, blocked = false } = {}) {
  const documentEvents = new Map(), windowEvents = new Map(), mediaEvents = new Map();
  const dataset = {}, writes = [];
  let reads = 0, queries = 0;
  const attrs = {};
  const picker = {
    value: 'system',
    dataset: {},
    innerHTML: '',
    attrs,
    setAttribute(name, value) { attrs[name] = value; },
    closest: selector => selector === '[data-theme-toggle]' ? picker : null,
  };
  const system = { matches: dark, addEventListener: (name, listener) => mediaEvents.set(name, listener) };
  const storage = {
    getItem(key) { assert.equal(key, 'miora.theme'); reads++; if (blocked) throw new Error('denied'); return stored; },
    setItem(key, value) { if (blocked) throw new Error('denied'); writes.push([key, value]); },
    removeItem(key) { if (blocked) throw new Error('denied'); writes.push([key, null]); },
  };
  runInNewContext(themeSource, {
    document: { documentElement: { dataset }, addEventListener: (name, listener) => documentEvents.set(name, listener), querySelectorAll() { queries++; return [picker]; } },
    window: { localStorage: storage, matchMedia: () => system, addEventListener: (name, listener) => windowEvents.set(name, listener) },
  });
  const click = () => documentEvents.get('click')({ target: picker });
  return { dataset, picker, writes, documentEvents, windowEvents, mediaEvents, ready: () => documentEvents.get('DOMContentLoaded')(), click,
    systemChange(value) { system.matches = value; mediaEvents.get('change')(); },
    storageChange(key, newValue) { windowEvents.get('storage')({ key, newValue }); },
    counts: () => ({ reads, queries }) };
}
await test('theme applies before DOM construction and follows system without writes or polling', () => {
  const fixture = themeFixture({ dark: true });
  assert.equal(fixture.dataset.theme, 'dark');
  assert.equal(fixture.dataset.themeMode, 'system');
  assert.deepEqual(fixture.counts(), { reads: 1, queries: 0 });
  fixture.ready();
  assert.match(fixture.picker.innerHTML, /^<svg class="icon"/);
  assert.equal(fixture.picker.attrs['aria-label'], '外观：跟随系统');
  fixture.systemChange(false);
  assert.equal(fixture.dataset.theme, 'light');
  assert.equal(fixture.picker.value, 'system');
  assert.deepEqual(fixture.writes, []);
  assert.deepEqual(fixture.counts(), { reads: 1, queries: 1 });
  assert.equal(fixture.mediaEvents.size, 1);
});
await test('explicit themes persist only appearance and system mode removes the override', () => {
  const fixture = themeFixture({ stored: 'light', dark: true });
  fixture.ready();
  assert.equal(fixture.dataset.theme, 'light');
  assert.equal(fixture.picker.attrs['aria-label'], '外观：浅色');
  fixture.click();
  fixture.systemChange(false);
  assert.equal(fixture.dataset.theme, 'dark');
  assert.equal(fixture.picker.attrs['aria-label'], '外观：深色');
  fixture.click();
  assert.equal(fixture.dataset.themeMode, 'system');
  assert.equal(fixture.dataset.theme, 'light');
  assert.deepEqual(fixture.writes, [['miora.theme', 'dark'], ['miora.theme', null]]);
});
await test('theme syncs across tabs without echoing storage events or accepting unknown modes', () => {
  const fixture = themeFixture({ stored: 'invalid', dark: false });
  fixture.ready();
  assert.equal(fixture.dataset.themeMode, 'system');
  fixture.storageChange('miora.theme', 'dark');
  assert.equal(fixture.picker.value, 'dark');
  fixture.storageChange('unrelated', 'light');
  assert.equal(fixture.picker.value, 'dark');
  fixture.storageChange(null, null);
  assert.equal(fixture.dataset.themeMode, 'system');
  assert.deepEqual(fixture.writes, []);
});
await test('denied browser storage does not prevent theme switching or system tracking', () => {
  const fixture = themeFixture({ blocked: true, dark: true });
  fixture.ready();
  fixture.click();
  assert.equal(fixture.dataset.theme, 'light');
  fixture.click();
  assert.equal(fixture.dataset.theme, 'dark');
  fixture.click();
  assert.equal(fixture.dataset.themeMode, 'system');
  assert.equal(fixture.dataset.theme, 'dark');
});
const dataURL = value => `data:text/javascript;base64,${Buffer.from(value).toString('base64')}`;
const iconsURL = dataURL(await source('icons.js'));
const icons = await import(iconsURL);
await test('every bundled AI mark has a fixed local image and engine identity', async () => {
  const files = (await readdir(new URL('ai-icons/', assets))).filter(file => file.endsWith('.svg'));
  assert.equal(files.length, 114);
  for (const file of files) {
    const brand = file.replace(/(?:-color)?\.svg$/, '');
    const html = icons.aiIcon(brand);
    assert.ok(html.includes(`src="/admin/assets/ai-icons/${file}"`), brand);
    assert.ok(html.includes(`data-ai-brand="${brand}"`), brand);
    assert.equal(html.includes('ai-icon-monochrome'), !file.endsWith('-color.svg'), brand);
    assert.equal(icons.channelBrand({ engine: brand }), brand);
  }
});
await test('AI marks use fixed local assets and invert only monochrome artwork', () => {
  for (const brand of ['openai', 'grok', 'ollama', 'openrouter']) {
    const html = icons.aiIcon(brand);
    assert.match(html, /class="ai-icon ai-icon-monochrome"/);
    assert.ok(html.includes(`src="/admin/assets/ai-icons/${brand}.svg"`));
    assert.match(html, /alt="" aria-hidden="true"/);
    assert.match(html, /width="22" height="22"/);
  }
  for (const brand of ['claude', 'gemini', 'gemma', 'deepseek', 'qwen', 'mistral', 'meta', 'azure', 'googlecloud', 'aws', 'cloudflare', 'siliconcloud', 'volcengine', 'kimi', 'zhipu', 'minimax', 'cohere', 'perplexity', 'doubao', 'hunyuan', 'wenxin', 'yi', 'stepfun', 'nvidia', 'huggingface']) {
    const html = icons.aiIcon(brand);
    assert.ok(html.includes(`/admin/assets/ai-icons/${brand}-color.svg`), brand);
    assert.ok(!html.includes('ai-icon-monochrome'), brand);
    assert.ok(html.includes(`data-ai-brand="${brand}"`));
  }
});
await test('unknown AI brands cannot become asset paths or markup', () => {
  for (const value of ['', 'unknown', '__proto__', 'constructor', '../../private', '"><img src=x onerror=bad>']) {
    assert.equal(icons.aiIcon(value), icons.icon('box'));
    assert.equal(icons.aiIcon(value, 'channels'), icons.icon('channels'));
  }
});
await test('channel brands follow the endpoint rather than the compatible API engine', () => {
  const endpoints = [
    ['api.openai.com', 'openai'], ['chatgpt.com', 'openai'], ['api.anthropic.com', 'claude'],
    ['generativelanguage.googleapis.com', 'gemini'], ['us-central1-aiplatform.googleapis.com', 'googlecloud'],
    ['resource.openai.azure.com', 'azure'], ['bedrock-runtime.us-east-1.amazonaws.com', 'aws'],
    ['api.deepseek.com', 'deepseek'], ['dashscope-intl.aliyuncs.com', 'qwen'], ['api.x.ai', 'grok'],
    ['api.mistral.ai', 'mistral'], ['openrouter.ai', 'openrouter'], ['api.moonshot.cn', 'kimi'],
    ['open.bigmodel.cn', 'zhipu'], ['api.minimax.io', 'minimax'], ['api.cohere.com', 'cohere'],
    ['api.perplexity.ai', 'perplexity'], ['api.siliconflow.cn', 'siliconcloud'],
    ['ark.cn-beijing.volces.com', 'volcengine'], ['api.cloudflare.com', 'cloudflare'],
    ['ollama.com', 'ollama'], ['router.huggingface.co', 'huggingface'], ['integrate.api.nvidia.com', 'nvidia'],
    ['hunyuan.tencentcloudapi.com', 'hunyuan'], ['qianfan.baidubce.com', 'wenxin'],
    ['api.stepfun.com', 'stepfun'], ['api.01.ai', 'yi'],
  ];
  for (const [host, brand] of endpoints) {
    assert.equal(icons.channelBrand({ engine: 'gpt', base_url: `https://${host}/v1` }), brand, host);
  }
  assert.equal(icons.channelBrand({ engine: 'gpt', base_url: 'https://API.DEEPSEEK.COM:443/v1', provider: 'OpenAI 兼容' }), 'deepseek');
});
await test('unknown compatible endpoints and misleading URL text stay generic', () => {
  for (const base_url of ['https://api.openai.com.evil.example/v1', 'https://api.openai.com@evil.example/v1', 'https://evil.example/api.openai.com', 'https://evil.example/?url=https://api.openai.com', 'https://private.example/v1', 'http://localhost:11434/v1', 'https://', '']) {
    assert.equal(icons.channelBrand({ engine: 'gpt', provider: '代理 A', base_url }), '', base_url);
  }
  assert.equal(icons.channelBrand({ engine: 'constructor' }), '');
});
await test('channel platform engines and explicit supplier names work without a public endpoint', () => {
  for (const [engine, brand] of [['azure', 'azure'], ['aws', 'aws'], ['vertex-claude', 'googlecloud'], ['openrouter', 'openrouter'], ['ollama', 'ollama'], ['claude', 'claude'], ['gemini', 'gemini'], ['codex', 'openai']]) {
    assert.equal(icons.channelBrand({ engine }), brand);
  }
  for (const [provider, brand] of [['硅基流动', 'siliconcloud'], ['火山引擎', 'volcengine'], ['豆包', 'doubao'], ['智谱 GLM', 'zhipu'], ['Kimi', 'kimi'], ['DeepSeek 官方', 'deepseek'], ['OpenAI 兼容', 'openai']]) {
    assert.equal(icons.channelBrand({ provider }), brand);
  }
  assert.equal(icons.channelBrand({ engine: 'openrouter', provider: 'Claude 代理' }), 'openrouter');
});
await test('model marks identify upstream families across provider namespaces and local model tags', () => {
  const families = [
    ['gpt-4.1', 'openai'], ['o3-mini', 'openai'], ['text-embedding-3-small', 'openai'], ['gpt-oss-20b', 'openai'],
    ['anthropic/claude-sonnet-4', 'claude'], ['models/gemini-2.5-pro', 'gemini'], ['google/gemma-3-27b-it', 'gemma'],
    ['deepseek-ai/DeepSeek-V3', 'deepseek'], ['Qwen/Qwen3-32B', 'qwen'], ['qwq-32b', 'qwen'], ['x-ai/grok-3', 'grok'],
    ['mistralai/mixtral-8x7b-instruct', 'mistral'], ['codestral-latest', 'mistral'], ['ollama/llama3.2:latest', 'meta'],
    ['moonshotai/kimi-k2', 'kimi'], ['z-ai/glm-4.5', 'zhipu'], ['MiniMax-M2', 'minimax'], ['command-r-plus', 'cohere'],
    ['sonar-pro', 'perplexity'], ['doubao-seed-1.6', 'doubao'], ['hunyuan-a13b', 'hunyuan'], ['ernie-4.5', 'wenxin'],
    ['yi-large', 'yi'], ['step-2', 'stepfun'],
  ];
  for (const [model, brand] of families) assert.equal(icons.modelBrand(model), brand, model);
});
await test('model family takes precedence over namespace and unknown public aliases stay generic', () => {
  assert.equal(icons.modelBrand('openrouter/anthropic/claude-sonnet-4'), 'claude');
  assert.equal(icons.modelBrand('google/custom-release'), 'gemini');
  assert.equal(icons.modelBrand('nvidia/nemotron-super'), 'nvidia');
  for (const value of ['', 'my-gpt-wrapper', 'grokking', 'llamafile', 'geminiature', 'public-fast', 'constructor/custom']) {
    assert.equal(icons.modelBrand(value), '', value);
  }
});
await test('additional language and embedding model families use their own brands', () => {
  assert.equal(icons.modelBrand('Meta-Llama-3.1-8B-Instruct'), 'meta');
  const cases = [
    ['jamba-1.5-large', 'ai21'], ['allenai/OLMo-2-7B', 'ai2'], ['Baichuan2-13B-Chat', 'baichuan'],
    ['internlm/internlm3-8b-instruct', 'internlm'], ['spark-4.0-ultra', 'spark'], ['SenseNova-V6', 'sensenova'],
    ['Skywork-13B', 'skywork'], ['LongCat-Flash-Chat', 'longcat'], ['mimo-v2-flash', 'xiaomimimo'],
    ['pangu-3.0', 'huawei'], ['microsoft/Phi-4-mini-instruct', 'microsoft'], ['granite-3.3-8b-instruct', 'ibm'],
    ['tiiuae/Falcon3-7B-Instruct', 'tii'], ['LiquidAI/LFM2-8B', 'liquid'], ['NousResearch/Hermes-3-Llama-3.1-8B', 'nousresearch'],
    ['arcee-ai/trinity-mini', 'arcee'], ['mercury-coder-small', 'inception'], ['solar-pro2', 'upstage'], ['reka-core', 'reka'],
    ['inflection-3-productivity', 'inflection'], ['BAAI/bge-m3', 'baai'], ['jina-embeddings-v3', 'jina'],
    ['voyage-3-large', 'voyage'], ['RWKV-7', 'rwkv'], ['deepcogito/cogito-v2-preview', 'deepcogito'],
    ['openchat-3.6', 'openchat'], ['databricks/dbrx-instruct', 'dbrx'], ['amazon.nova-pro-v1:0', 'nova'],
    ['luminous-supreme', 'alephalpha'], ['CohereForAI/aya-expanse-32b', 'aya'], ['Nemotron-Nano-9B', 'nvidia'],
  ];
  for (const [model, brand] of cases) assert.equal(icons.modelBrand(model), brand, model);
});
await test('image video and audio model families keep distinct artwork', () => {
  const cases = [
    ['stable-diffusion-xl', 'stability'], ['sd3.5-large', 'stability'], ['black-forest-labs/FLUX.1-dev', 'flux'],
    ['ideogram-v3', 'ideogram'], ['recraft-v3', 'recraft'], ['midjourney-v6', 'midjourney'], ['gen4_turbo', 'runway'],
    ['pika-2.2', 'pika'], ['kling-v2-master', 'kling'], ['vidu-q1', 'vidu'], ['ray-2', 'luma'],
    ['eleven_multilingual_v2', 'elevenlabs'], ['fish-speech-1.5', 'fishaudio'], ['suno-v4', 'suno'], ['udio-v1.5', 'udio'],
    ['minimax-hailuo-02', 'hailuo'], ['jimeng-3.0', 'jimeng'], ['pixverse-v4', 'pixverse'], ['bria-3.2', 'briaai'],
    ['tripo-v2.5', 'tripo'], ['meshy-5', 'meshy'], ['universal-2', 'assemblyai'], ['xtts-v2', 'coqui'],
    ['THUDM/CogVideoX-5b', 'cogvideo'], ['THUDM/CogView4-6B', 'cogview'], ['Kwai-Kolors/Kolors', 'kolors'],
    ['sora-2', 'sora'], ['HunyuanVideo', 'hunyuan'],
  ];
  for (const [model, brand] of cases) assert.equal(icons.modelBrand(model), brand, model);
});
const uiURL = dataURL((await source('ui.js')).replace("'./icons.js'", JSON.stringify(iconsURL)));
const ui = await import(uiURL);
const modelsURL = dataURL((await source('models-editor.js')).replace("'./icons.js'", JSON.stringify(iconsURL)).replace("'./ui.js'", JSON.stringify(uiURL)));
const models = await import(modelsURL);
const apiURL = dataURL(await source('api.js'));
const api = await import(apiURL);
const keyManagerURL = dataURL((await source('key-manager.js')).replace("'./icons.js'", JSON.stringify(iconsURL)).replace("'./ui.js'", JSON.stringify(uiURL)).replace("'./api.js'", JSON.stringify(apiURL)));
const keyManager = await import(keyManagerURL);
const usageDashboard = await import(dataURL((await source('usage-dashboard.js')).replace("'./icons.js'", JSON.stringify(iconsURL)).replace("'./ui.js'", JSON.stringify(uiURL))));
await test('usage detail names unknown historical channels without treating them as a model', async () => {
  const captures = [];
  const plain = (await source('usage-dashboard.js')).replace(/^import .*;\r?\n/gm, '').replace(/^export /gm, '');
  const open = runInNewContext(plain + '\nopenUsageDetail', { ...ui, dialog: options => captures.push(options) });
  open({ channel: '', request_count: 1 });
  open({ model: 'public-m', request_count: 1 });
  assert.deepEqual(captures.map(options => options.title), ['未记录渠道', 'public-m']);
});
await test('usage drilldown carries exact time, channel and public-model scope', () => {
  const href = usageDashboard.usageLogLink({ model: 'public/a & b' }, { from: 0, to: 86400 }, { channel: '渠道/一' });
  const params = new URLSearchParams(href.split('?')[1]);
  assert.equal(params.get('model'), 'public/a & b');
  assert.equal(params.get('channel'), '渠道/一');
  assert.equal(params.get('from_ms'), '0');
  assert.equal(params.get('to_ms'), '86400000');
  const channel = new URLSearchParams(usageDashboard.usageLogLink({ channel: 'b' }, { from: 7, to: 9 }, { channel: 'a', model: 'm' }).split('?')[1]);
  assert.equal(channel.get('channel'), 'b');
  assert.equal(channel.get('model'), 'm');
});
await test('channel breakdown uses its own rows, escaping and truncation state', () => {
  const data = { usage: [{ model: 'not-this-row' }], channels: [{ channel: '<channel>', request_count: 1 }], channels_truncated: true, from: 1, to: 2 };
  const html = usageDashboard.usageBreakdown(data, { group: 'channel', model: 'm' });
  assert.match(html, /渠道对比/);
  assert.match(html, /&lt;channel&gt;/);
  assert.ok(!html.includes('not-this-row'));
  assert.match(html, /仅显示前 500 个/);
  assert.match(html, /model=m&amp;channel=%3Cchannel%3E/);
});
await test('trend buckets clip edge drilldown to the selected window and preserve unknown versus zero', () => {
  const data = { from: 3500, to: 4000, bucket_seconds: 3600, trend: [
    { bucket_start: 0, request_count: 1, success_count: 1, failure_count: 0, estimated_cost: null },
    { bucket_start: 3600, request_count: 1, success_count: 1, failure_count: 0, estimated_cost: 0 },
  ] };
  const html = usageDashboard.usageTrend(data, { model: 'm' });
  assert.match(html, /from_ms=3500000&amp;to_ms=3600000/);
  assert.match(html, /from_ms=3600000&amp;to_ms=4000000/);
  assert.equal((html.match(/<rect /g) || []).length, 2);
  const costs = usageDashboard.usageTrend(data, {}, 'estimated_cost');
  assert.equal((costs.match(/<rect /g) || []).length, 1);
  assert.match(costs, /\$0\.000000/);
  assert.ok(!costs.includes('NaN') && !costs.includes('Infinity'));
  data.trend[1].estimated_cost = null;
  assert.match(usageDashboard.usageTrend(data, {}, 'estimated_cost'), /未报告该指标/);
});
await test('usage renders the API contract instead of hiding malformed arrays as empty data', () => {
  assert.throws(() => usageDashboard.usageBreakdown({ usage: null }, { group: 'model' }));
  assert.throws(() => usageDashboard.usageTrend({ trend: {} }, {}));
});
await test('usage dashboard: unknown, reported zero and tiny costs remain distinct', () => {
  assert.equal(usageDashboard.money(null), '—');
  assert.equal(usageDashboard.money(0), '$0.000000');
  assert.equal(usageDashboard.money(0.000252), '$0.000252');
  assert.equal(usageDashboard.percent(null), '—');
  assert.equal(usageDashboard.percent(0), '0.0%');
  assert.equal(usageDashboard.percent(0.6), '60.0%');
});
await test('usage dashboard: model markup is escaped and partial reporting is visible', () => {
  const row = { model: '<img onerror="bad">', request_count: 3, input_tokens: 100, input_samples: 1, cache_miss_tokens: 40, estimated_cost: null };
  const html = usageDashboard.usageDashboard({ available: true, summary: row, usage: [row], trend: [] }, { days: 1, model: '', channel: '' });
  assert.ok(!html.includes('<img'));
  assert.ok(html.includes('&lt;img'));
  assert.ok(html.includes('已报告 1 / 3'));
  assert.ok(html.includes('供应商账单'));
  assert.ok(html.includes('logs?model='));
});
const pluginManager = await import(dataURL((await source('plugin-manager.js')).replace("'./icons.js'", JSON.stringify(iconsURL)).replace("'./ui.js'", JSON.stringify(uiURL)).replace("'./api.js'", JSON.stringify(apiURL))));
const pluginPackage = () => ({ format: 'miora.plugin.v1', manifest: { id: 'echo', name: 'Echo', version: '1.0.0', api_version: 1, config_schema: [{ key: 'credential', type: 'string', required: true, secret: true }], capabilities: { endpoints: ['/v1/chat/completions'] } }, endpoint: 'http://127.0.0.1:8765', config: { enabled: false, limit: 0 } });
const parsePlugin = value => pluginManager.parsePluginFile(JSON.stringify(value));
await test('plugin upload: native metadata keeps falsy defaults without requiring channel credentials', () => {
  const parsed = parsePlugin(pluginPackage());
  assert.deepEqual(parsed.config, { enabled: false, limit: 0 });
  assert.equal(parsed.manifest.config_schema[0].required, true);
  assert.equal(parsed.config.credential, undefined);
});
await test('plugin upload: foreign formats, invalid JSON and missing preview metadata are rejected', () => {
  for (const patch of [{ format: 'foreign.v1' }, { manifest: {} }, { manifest: { id: 'echo', name: 3, version: '1' } }]) assert.throws(() => parsePlugin({ ...pluginPackage(), ...patch }));
  assert.throws(() => pluginManager.parsePluginFile('import os'));
});
await test('plugin preview delegates service trust and schema validation to the install boundary', () => {
  for (const patch of [{ extra: true }, { endpoint: 'javascript:alert(1)' }, { endpoint: 'https://user:secret@example.com' }, { service_auth: { type: 'future' } }, { config: false }]) {
    const value = { ...pluginPackage(), ...patch };
    assert.deepEqual(parsePlugin(value), value);
  }
});
await test('plugin preview does not revalidate schema and list metadata is escaped', () => {
  const value = pluginPackage();
  value.manifest.config_schema.push(value.manifest.config_schema[0]);
  assert.equal(parsePlugin(value).manifest.config_schema.length, 2);
  value.manifest.name = '<img onerror="bad">';
  const html = pluginManager.pluginList([{ kind: 'service', id: 'echo', enabled: true, manifest: value.manifest }]);
  assert.ok(html.includes('&lt;img'));
  assert.ok(!html.includes('<img'));
  assert.ok(!html.includes('edit-plugin'));
  assert.ok(html.includes('upload-plugin-update'));
});
await test('plugin upload: installed plugins and built-in adapters have separate lists', () => {
  const items = [{ kind: 'service', id: 'uploaded-echo' }, { kind: 'builtin', id: 'builtin-example' }];
  const installed = pluginManager.pluginList(items);
  const builtin = pluginManager.pluginList(items, 'builtin');
  assert.ok(installed.includes('uploaded-echo') && !installed.includes('builtin-example'));
  assert.ok(builtin.includes('builtin-example') && !builtin.includes('uploaded-echo'));
  assert.ok(!builtin.includes('toggle-plugin'));
});
const parametersURL = dataURL((await source('parameters-editor.js')).replace("'./icons.js'", JSON.stringify(iconsURL)).replace("'./ui.js'", JSON.stringify(uiURL)));
const parameters = await import(parametersURL);
const channel = await import(dataURL((await source('channel-editor.js')).replace("'./icons.js'", JSON.stringify(iconsURL)).replace("'./ui.js'", JSON.stringify(uiURL)).replace("'./api.js'", JSON.stringify(apiURL)).replace("'./models-editor.js'", JSON.stringify(modelsURL)).replace("'./parameters-editor.js'", JSON.stringify(parametersURL)).replace("'./key-manager.js'", JSON.stringify(keyManagerURL))));
const logSettingsURL = dataURL((await source('log-settings.js')).replace("'./icons.js'", JSON.stringify(iconsURL)).replace("'./ui.js'", JSON.stringify(uiURL)).replace("'./api.js'", JSON.stringify(apiURL)));
const logSettings = await import(logSettingsURL);
const requestLogs = await import(dataURL((await source('request-logs.js')).replace("'./icons.js'", JSON.stringify(iconsURL)).replace("'./ui.js'", JSON.stringify(uiURL)).replace("'./api.js'", JSON.stringify(apiURL)).replace("'./log-settings.js'", JSON.stringify(logSettingsURL))));
await test('log hash retains millisecond scope and rejects invalid time at the input boundary', () => {
  assert.deepEqual(requestLogs.logFiltersFromHash('#logs?model=public%2Fm&channel=a%26b&from_ms=0&to_ms=86400123'), { model: 'public/m', channel: 'a&b', from_ms: 0, to_ms: 86400123 });
  assert.deepEqual(requestLogs.logFiltersFromHash('#logs'), {});
  assert.throws(() => requestLogs.logFiltersFromHash('#logs?from_ms=NaN'), /日期与时间/);
  assert.throws(() => requestLogs.logFiltersFromHash('#logs?from_ms=10&to_ms=10'), /早于/);
  const href = usageDashboard.usageLogLink({ channel: 'a' }, { from: 3500, to: 4000 }, { model: 'm' });
  assert.deepEqual(requestLogs.logFiltersFromHash(href), { model: 'm', channel: 'a', from_ms: 3500000, to_ms: 4000000 });
});
const form = values => ({ elements: { namedItem(name) { return values[name]; } } });

await test('channel pricing: clearing saved rates sends null without losing model overrides', () => {
  const original = { input: 2, cache_read: .2, models: { premium: { input: 9 } } };
  const values = { input: '0', output: '8', cache_read: '', cache_write: '', cache_write_1h: '' };
  assert.deepEqual(models.readUsagePricing(values, original), { input: 0, output: 8, cache_read: null, models: original.models });
  assert.equal(original.cache_read, .2);
  assert.equal(models.readUsagePricing({}), undefined);
  assert.throws(() => models.readUsagePricing({ input: '-1' }), /单价/);
  assert.throws(() => models.readUsagePricing({ input: 'Infinity' }), /单价/);
});

await test('per-model pricing keeps models and channels independent, including zero and missing prices', () => {
  const rows = models.modelsToRows([{ upstream: 'public' }, 'other']);
  const first = models.createModelPricing(), second = models.createModelPricing();
  first.entry(rows[0]).values.input = '2';
  first.entry(rows[0]).values.output = '0';
  first.entry(rows[1]).values.input = '9';
  second.entry(rows[0]).values.input = '7';
  assert.deepEqual(first.value(rows), { models: { public: { input: 2, output: 0 }, other: { input: 9 } } });
  assert.deepEqual(second.value(rows), { models: { public: { input: 7 } } });
  first.entry(rows[0]).values.input = '-1';
  assert.throws(() => first.value(rows), /public.*单价/);
});

await test('model pricing preserves legacy defaults without materializing an untouched price catalog', () => {
  const original = { input: 2, output: 8, models: { public: { input: 3, cache_read: null }, unused: { input: 5 } } };
  const rows = models.modelsToRows([{ upstream: 'public' }, 'new']);
  const pricing = models.createModelPricing(original);
  assert.deepEqual(pricing.value(rows), original);
  assert.equal(pricing.entry(rows[0]).values.cache_read, '');
  pricing.entry(rows[1]).values.input = '';
  assert.deepEqual(pricing.value(rows).models.new, { input: null });
  assert.deepEqual(original.models.public, { input: 3, cache_read: null });
  assert.equal(original.models.new, undefined);
});

await test('model pricing treats prototype-like model names as ordinary names', () => {
  const pricing = models.createModelPricing();
  const rows = [{ upstream: '__proto__', alias: '' }, { upstream: 'constructor', alias: '' }];
  pricing.entry(rows[0]).values.input = '1';
  pricing.entry(rows[1]).values.input = '2';
  const result = pricing.value(rows);
  assert.equal(Object.hasOwn(result.models, '__proto__'), true);
  assert.deepEqual(result.models.__proto__, { input: 1 });
  assert.deepEqual(result.models.constructor, { input: 2 });
  assert.equal(Object.getPrototypeOf(result.models), Object.prototype);
});

await test('renaming an unpriced model cannot silently pick up a different saved model price', () => {
  const pricing = models.createModelPricing({ models: { after: { input: 99 } } });
  const row = { upstream: 'before', alias: '' };
  assert.equal(pricing.entry(row).values.input, '');
  row.alias = 'after';
  assert.deepEqual(pricing.value([row]), { models: {} });
});

await test('renaming or removing model rows moves their rates and preserves unrelated catalog entries', () => {
  const rows = models.modelsToRows([{ upstream: 'before' }, 'remove']);
  const pricing = models.createModelPricing({ models: { before: { input: 4 }, remove: { output: 6 }, unused: { input: 9 } } });
  rows.forEach(row => pricing.entry(row));
  rows[0].alias = 'after';
  assert.deepEqual(pricing.value([rows[0]]), { models: { after: { input: 4 }, unused: { input: 9 } } });
  rows[0].upstream = 'changed-upstream';
  assert.equal(pricing.value([rows[0]]).models.after.input, 4);
  const copy = { upstream: 'changed-upstream', alias: 'second-alias' };
  pricing.copy(rows[0], copy);
  pricing.entry(copy).values.input = '10';
  assert.deepEqual(pricing.value([rows[0], copy]).models, { after: { input: 4 }, 'second-alias': { input: 10 }, unused: { input: 9 } });
  assert.deepEqual(pricing.value([]), { models: { unused: { input: 9 } } });
});

await test('channel editor close callback can access its OAuth timer outside onMount', async () => {
  const editorSource = (await source('channel-editor.js')).replace(/^import .*;\r?\n/gm, '').replace(/^export /gm, '');
  let options, cleared = 0;
  const openEditor = runInNewContext(editorSource + '\nopenChannelEditor', {
    ...ui, parseGroups: keyManager.parseGroups, icon: () => '',
    api: { get: async () => ({}) },
    dialog(value) { options = value; return {}; },
    clearInterval() { cleared++; },
  });
  await openEditor({});
  assert.doesNotThrow(() => options.onClose());
  assert.equal(cleared, 1);
});

await test('channel draft reuses validated models and parses parameter policy only once', async () => {
  const editorSource = await source('channel-editor.js');
  const draftSource = editorSource.slice(editorSource.indexOf('  function draftChannel('), editorSource.indexOf('  function dto('));
  const elements = { extra: { value: '{}' }, protocol_mode: { value: 'auto' }, channel_groups: { value: 'default' }, allow_byok: { checked: false }, retry_empty: { checked: false }, reasoning_history: { value: 'preserve' }, reasoning_suffixes: { checked: false } };
  let policyReads = 0, modelReads = 0, rejected = false;
  const tabs = [], policy = { defaults: { temperature: .5 } }, model = ['public'];
  const draft = runInNewContext(draftSource + '\ndraftChannel', {
    form: { elements }, parseJSON: ui.parseJSON, parseGroups: keyManager.parseGroups, config: {}, connection: () => ({ engine: 'gpt' }),
    parametersEditor: { value() { policyReads++; if (rejected) throw new Error('invalid policy'); return policy; } },
    modelsEditor: { snapshot() { modelReads++; return []; }, pricing: () => ({ models: { public: { input: 2 } } }) }, rowsToModels: models.rowsToModels,
    activateTab: tab => tabs.push(tab),
  });
  const result = draft({ validate: true, model });
  assert.equal(result.model, model);
  assert.equal(result.preferences.parameter_policy, policy);
  assert.deepEqual(result.preferences.usage_pricing, { models: { public: { input: 2 } } });
  assert.equal(policyReads, 1);
  assert.equal(modelReads, 0);
  assert.equal((editorSource.match(/parametersEditor\.value\(\)/g) || []).length, 1);
  rejected = true;
  assert.throws(() => draft({ validate: true, model }), /invalid policy/);
  assert.deepEqual(tabs, ['parameters']);
});

await test('system settings distinguish persistent saves from read-only remote sources', async () => {
  const appSource = await source('app.js');
  const settingsSource = appSource.slice(appSource.indexOf('const settingsLabels'), appSource.indexOf('function settingsMutation('));
  for (const writable of [true, false]) {
    const render = runInNewContext(settingsSource + '\nsettings', {
      read: async () => ({ writable, revision: 'fixture', effective: { preferences: {} } }), state: {},
      head: () => '', settingsNav: () => '', field: () => '', checkbox: () => '', jsonDetails: () => '',
      icon: () => '', esc: String, notice: message => `<p>${message}</p>`,
    });
    const html = await render();
    assert.equal(html.includes('<fieldset class="settings-fields" disabled>'), !writable);
    assert.equal(html.includes('id="validate-settings" disabled'), !writable);
    assert.equal(html.includes('type="submit" disabled'), !writable);
    assert.equal(html.includes('远程配置为只读'), !writable);
    assert.ok(html.includes('保存设置'));
    assert.ok(!html.includes('重启后恢复') && !html.includes('保存运行时设置'));
  }
});

await test('settings save sends one PATCH; explicit validation sends only POST', async () => {
  const appSource = await source('app.js');
  const saveSource = appSource.slice(appSource.indexOf('async function saveSettings('), appSource.indexOf('const standalonePages'));
  const calls = [], payload = { revision: 'fixture', changes: [] };
  const errorBox = { hidden: true, focus() {} }, successBox = { hidden: true };
  const settingsForm = { dataset: {}, reportValidity: () => true, setAttribute() {}, removeAttribute() {}, querySelectorAll: () => [],
    querySelector: selector => selector === '#settings-error' ? errorBox : successBox };
  let renders = 0;
  const save = runInNewContext(saveSource + '\nsaveSettings', {
    settingsMutation: () => payload, toast() {}, render: async () => { renders++; },
    api: { post: async (path, body) => calls.push(['POST', path, body]), patch: async (path, body) => calls.push(['PATCH', path, body]) },
  });
  await save(settingsForm, false);
  assert.deepEqual(calls, [['PATCH', '/settings', payload]]);
  assert.equal(renders, 1);
  calls.length = 0;
  await save(settingsForm, true);
  assert.deepEqual(calls, [['POST', '/settings/validate', payload]]);
  assert.equal(renders, 1);
  assert.equal(successBox.hidden, false);
  assert.equal(settingsForm.dataset.busy, undefined);
});

await test('admin cleanup: cached fmt matches the previous expression for numbers and unknowns', () => {
  const previousFmt = value => typeof value === 'number' && Number.isFinite(value) ? new Intl.NumberFormat('zh-CN').format(value) : '—';
  const values = [0, -0, 1, -1, 1234, 1234.56789, -9876543.21, 1e21, Number.MAX_SAFE_INTEGER, Number.MIN_VALUE, NaN, Infinity, -Infinity, null, undefined, '9', false, {}, 1n];
  assert.deepEqual(values.map(ui.fmt), values.map(previousFmt));
});
await test('admin cleanup: real ui module constructs one formatter for repeated numeric values', async t => {
  const OriginalNumberFormat = Intl.NumberFormat;
  let constructions = 0;
  const values = Array.from({ length: 250 }, (_, index) => index * 1234.567 - 100);
  Intl.NumberFormat = new Proxy(OriginalNumberFormat, { construct(target, args) { constructions++; return new target(...args); } });
  try {
    // A fresh module instance of the actual ui.js; only its icon import URL was resolved above.
    const measuredUI = await import(`${uiURL}#number-format-construction-probe`);
    const current = values.map(measuredUI.fmt);
    const currentConstructions = constructions;
    assert.equal(currentConstructions, 1);
    constructions = 0;
    const previous = values.map(value => typeof value === 'number' && Number.isFinite(value) ? new Intl.NumberFormat('zh-CN').format(value) : '—');
    assert.equal(constructions, values.length);
    assert.deepEqual(current, previous);
    t.diagnostic(`${process.version}; 250 identical finite inputs and equal outputs; NumberFormat constructions: previous=${constructions}, current=${currentConstructions} (counts, not page-speed measurements)`);
  } finally { Intl.NumberFormat = OriginalNumberFormat; }
});
await test('admin cleanup: failed-session retry uses one delegated pointer or keyboard click', async () => {
  const makeNode = () => ({
    listeners: new Map(), attributes: new Map(), disabled: false, hidden: false, innerHTML: '', textContent: '',
    classList: { remove() {}, contains() { return false; } },
    addEventListener(type, listener) { if (!this.listeners.has(type)) this.listeners.set(type, []); this.listeners.get(type).push(listener); },
    setAttribute(name, value) { this.attributes.set(name, value); },
    querySelectorAll() { return []; },
  });
  const retry = makeNode(); retry.dataset = { action: 'refresh' }; retry.closest = () => retry;
  const main = makeNode(); main.querySelector = selector => selector === '[data-action="refresh"]' ? retry : null;
  const nodes = new Map([['#main-content', main]]);
  const document = {
    querySelector(selector) { if (!nodes.has(selector)) nodes.set(selector, makeNode()); return nodes.get(selector); },
    querySelectorAll() { return []; }, addEventListener() {},
  };
  let reloads = 0;
  const window = { location: { reload() { reloads++; }, hash: '#overview' }, matchMedia() { return { matches: false, addEventListener() {} }; }, addEventListener() {} };
  const app = (await source('app.js')).replace(/^import .*;\r?\n/gm, '');
  // Run the real startup/error/delegated-handler code, with no network or browser.
  await runInNewContext(`(async () => {\n${app}\n})()`, {
    document, window, ...ui, icon: icons.icon, mountIcons() {},
    api: { async session() { throw new Error('session fixture failure'); } },
    mountWorkspace() {}, confirmWorkspaceLeave() { return true; }, mountBackendLogs() {}, mountTesting() {}, mountPlayground() {}, mountKeys() {}, mountRequestLogs() {}, mountLogSettings() {}, settingsNav() { return ''; }, logsNav() { return ''; }, openChannelEditor() {},
  });
  assert.equal(document.querySelector('#session-state').textContent, '会话验证失败');
  assert.match(main.innerHTML, /session fixture failure/);
  assert.match(main.innerHTML, /data-action="refresh"/);
  assert.match(main.innerHTML, /href="\/login"/);
  assert.equal((retry.listeners.get('click') || []).length, 0, 'no duplicate target listener');
  assert.equal(main.listeners.get('click').length, 1);
  // Native buttons dispatch bubbling click events for both pointer and Enter/Space activation.
  // Browser key synthesis itself remains for the parent-built live smoke.
  for (const detail of [1, 0]) {
    const before = reloads;
    const event = { target: detail ? { closest() { return retry; } } : retry, detail };
    for (const listener of retry.listeners.get('click') || []) await listener(event);
    for (const listener of main.listeners.get('click')) await listener(event);
    assert.equal(reloads - before, 1);
    assert.equal(retry.disabled, false);
  }
});

const logOriginal = { enabled: true, capture_fields: [], body_retention_hours: 0, row_retention: { mode: 'manual', days: 30 }, limits: { per_request_bytes: 1048576, total_body_bytes: 67108864 }, runtime: { file_max_bytes: 10485760, retained_files: 3 } };
const logValues = { enabled: true, capture_fields: ['request_body'], body_retention_hours: '0', row_mode: 'manual', row_days: '', per_request_kib: '1024', total_body_mib: '0', runtime_file_mib: '10', runtime_files: '3' };
await test('log UI: eight exact capture fields are available without invented stages', () => {
  assert.equal(logSettings.CAPTURE_FIELDS.length, 8);
  assert.equal(new Set(logSettings.CAPTURE_FIELDS.map(item => item.key)).size, 8);
  assert.deepEqual(logSettings.CAPTURE_FIELDS.map(item => item.key), ['request_headers', 'request_body', 'upstream_request_headers', 'upstream_request_body', 'upstream_response_headers', 'upstream_response_body', 'response_headers', 'response_body']);
});
await test('log UI: capacity units round-trip exact bytes and zero is not blank', () => {
  assert.equal(logSettings.unitsToBytes('1.5', 'KiB'), 1536);
  assert.equal(logSettings.unitsToBytes('0', 'MiB'), 0);
  assert.equal(logSettings.unitsToBytes(logSettings.bytesToUnits(1, 'KiB'), 'KiB'), 1);
  assert.equal(logSettings.unitsToBytes('16', 'MiB'), 16777216);
  assert.throws(() => logSettings.unitsToBytes('', 'MiB'), /0 与未填写不同/);
  assert.throws(() => logSettings.unitsToBytes('-1', 'KiB'), /非负/);
  assert.throws(() => logSettings.unitsToBytes('0.1', 'KiB'), /整数字节/);
});
await test('log UI: settings keep explicit zero retention and budget, omit unmanaged runtime', () => {
  const settings = logSettings.buildLogSettings(logOriginal, logValues, false);
  assert.equal(settings.body_retention_hours, 0);
  assert.equal(settings.limits.total_body_bytes, 0);
  assert.equal(settings.row_retention.days, 30);
  assert.equal(Object.hasOwn(settings, 'runtime'), false);
  assert.deepEqual(logOriginal.capture_fields, []);
  assert.deepEqual(settings.capture_fields, ['request_body']);
});
await test('log UI: backend numeric bounds reject per-request zero and invalid day retention', () => {
  assert.throws(() => logSettings.buildLogSettings(logOriginal, { ...logValues, per_request_kib: '0' }, false), /0 不是无限制/);
  assert.throws(() => logSettings.buildLogSettings(logOriginal, { ...logValues, per_request_kib: '16385' }, false), /16384/);
  assert.throws(() => logSettings.buildLogSettings(logOriginal, { ...logValues, row_mode: 'days', row_days: '0' }, false), /至少 1/);
  assert.throws(() => logSettings.buildLogSettings(logOriginal, { ...logValues, row_mode: 'days', row_days: '36501' }, false), /36500/);
  assert.equal(logSettings.buildLogSettings(logOriginal, { ...logValues, row_mode: 'forever', row_days: '' }, false).row_retention.days, 30);
});
await test('log UI: managed runtime converts units and enforces inclusive file-count bounds', () => {
  const settings = logSettings.buildLogSettings(logOriginal, { ...logValues, runtime_file_mib: '1024', runtime_files: '32' }, true);
  assert.deepEqual(settings.runtime, { file_max_bytes: 1073741824, retained_files: 32 });
  assert.throws(() => logSettings.buildLogSettings(logOriginal, { ...logValues, runtime_files: '0' }, true), /至少 1/);
  assert.throws(() => logSettings.buildLogSettings(logOriginal, { ...logValues, runtime_files: '33' }, true), /32/);
  assert.throws(() => logSettings.buildLogSettings(logOriginal, { ...logValues, runtime_file_mib: '0' }, true), /1 KiB/);
});
await test('log UI: disabled, zero-hour, zero-budget and no-field explanations are distinct', () => {
  assert.match(logSettings.captureExplanation(logOriginal, false), /后端当前不可用/);
  assert.match(logSettings.captureExplanation({ ...logOriginal, enabled: false }), /请求日志已关闭/);
  assert.match(logSettings.captureExplanation(logOriginal), /0 小时/);
  assert.match(logSettings.captureExplanation({ ...logOriginal, body_retention_hours: 24, limits: { total_body_bytes: 0 } }), /没有原始内容存储容量/);
  assert.match(logSettings.captureExplanation({ ...logOriginal, body_retention_hours: 24 }), /未选择任何捕获字段/);
});
await test('log UI: exact filters keep epoch zero and reject invalid HTTP/time ranges', () => {
  const filters = requestLogs.parseLogFilters({ from: '1970-01-01T00:00:00Z', to: '1970-01-02T00:00:00Z', status: '200', channel: ' c ', model: ' m ', key_id: 'unsupported' });
  assert.deepEqual(filters, { channel: 'c', model: 'm', status: 200, from_ms: 0, to_ms: 86400000 });
  assert.match(requestLogs.logQuery(filters, 2, 50), /from_ms=0/);
  assert.match(requestLogs.logQuery(filters, 2, 50), /page=2/);
  assert.throws(() => requestLogs.parseLogFilters({ status: '2xx' }), /精确状态码/);
  assert.throws(() => requestLogs.parseLogFilters({ from: '2026-01-02', to: '2026-01-01' }), /早于/);
  assert.throws(() => requestLogs.parseLogFilters({ from: 'not-a-date' }), /日期与时间/);
});
await test('log UI: content paths constrain fields and preserve actual attempt identifiers', () => {
  const path = requestLogs.logContentPath('../id?x=<svg>', 'upstream_response_body', 7);
  assert.match(path, /^\/request-logs\/\.\.%2Fid%3Fx%3D%3Csvg%3E\/content\?/);
  assert.match(path, /attempt=7/);
  assert.match(path, /limit=65536/);
  assert.equal(requestLogs.logContentPath('id', 'request_body', null, true), '/request-logs/id/download?field=request_body');
  assert.throws(() => requestLogs.logContentPath('id', 'http://remote.invalid', null), /未知日志内容字段/);
  assert.throws(() => requestLogs.logContentPath('id', 'response_body', -1), /尝试序号/);
});
await test('log UI: partial storage can be available, but expired cleaned and evicted cannot', () => {
  assert.equal(requestLogs.isFieldAvailable({ available: true, availability: 'available', capture_state: 'truncated', complete: false }), true);
  for (const availability of ['expired', 'cleaned', 'cleared', 'evicted', 'missing']) assert.equal(requestLogs.isFieldAvailable({ available: true, availability }), false);
  assert.equal(requestLogs.isFieldAvailable({ available: false, availability: 'available' }), false);
  assert.equal(requestLogs.isFieldAvailable({ available: true }), false);
});
await test('log UI: cleanup preview freezes explicit intent without inventing execution counts', () => {
  const filters = { from_ms: 0, request_id: 'r1' };
  assert.deepEqual(requestLogs.cleanupRequest('clear_fields', ['response_body', 'response_body'], filters), { dry_run: true, action: 'clear_fields', fields: ['response_body'], filters });
  assert.deepEqual(requestLogs.cleanupRequest('delete_rows', ['response_body'], filters), { dry_run: true, action: 'delete_rows', fields: [], filters });
  assert.throws(() => requestLogs.cleanupRequest('clear_fields', [], {}), /至少一个/);
  assert.throws(() => requestLogs.cleanupRequest('clear_fields', ['secret_database'], {}), /有效/);
  assert.throws(() => requestLogs.cleanupRequest('delete_stats', [], {}), /清理动作/);
});
await test('log UI: captured completeness never implies client receipt and missing metrics are not zero', () => {
  assert.equal(requestLogs.fieldCompletenessLabel(true), '捕获时完整');
  assert.match(requestLogs.fieldCompletenessLabel(false), /未完整/);
  assert.equal(requestLogs.completionLabel(true, true), '转发至 HTTP 栈完成');
  assert.equal(requestLogs.completionLabel(null, true), '未观察');
  assert.equal(requestLogs.formatBytes(0), '0 B');
  assert.equal(requestLogs.formatBytes(null), '—');
  assert.equal(requestLogs.formatDuration(0), '0 ms');
  assert.equal(requestLogs.formatDuration(undefined), '—');
  assert.notEqual(requestLogs.formatLogTime(0), '—');
  assert.equal(requestLogs.formatLogTime(NaN), '—');
});

await test('parameter editor: channel patch preserves unknown nested policy fields and falsy values', () => {
  const initial = { vendor_extension: { enabled: false, value: null }, defaults: { temperature: 0 }, channel: { custom: 'keep', set: { generationConfig: { vendor_extra: false, topP: 0 }, other: '' }, remove: ['/unused'] }, models: [{ match: { requested_model: 'public-a' }, set: { temperature: 0.1 }, extra: true }] };
  const next = parameters.patchParameter(initial, '/generationConfig/temperature', 'set', 0);
  assert.equal(next.channel.set.generationConfig.temperature, 0);
  assert.equal(next.channel.set.generationConfig.vendor_extra, false);
  assert.equal(next.channel.set.generationConfig.topP, 0);
  assert.equal(next.channel.set.other, '');
  assert.equal(next.channel.custom, 'keep');
  assert.deepEqual(next.vendor_extension, initial.vendor_extension);
  assert.deepEqual(next.models, initial.models);
  assert.deepEqual(next.defaults, initial.defaults);
  assert.equal(initial.channel.set.generationConfig.temperature, undefined);
});
await test('parameter editor: inheritance deletes override while removal uses a separate pointer', () => {
  const initial = { channel: { set: { temperature: 0, untouched: false }, remove: [] } };
  const inherited = parameters.patchParameter(initial, '/temperature', 'inherit');
  assert.equal(Object.hasOwn(inherited.channel.set, 'temperature'), false);
  assert.deepEqual(inherited.channel.remove, []);
  assert.equal(inherited.channel.set.untouched, false);
  const removed = parameters.patchParameter(initial, '/temperature', 'remove');
  assert.deepEqual(removed.channel.remove, ['/temperature']);
  assert.equal(Object.hasOwn(removed.channel.set, 'temperature'), false);
});
await test('parameter editor: zero false empty string and null remain actual set values', () => {
  for (const value of [0, false, '', null]) {
    const policy = parameters.patchParameter({}, '/temperature', 'set', value);
    assert.deepEqual(parameters.parameterState(policy, '/temperature'), { mode: 'set', value });
    assert.deepEqual(policy.channel.remove, []);
  }
  assert.equal(parameters.parseParameterValue('0'), 0);
  assert.equal(parameters.parseParameterValue('false'), false);
  assert.equal(parameters.parseParameterValue(''), '');
  assert.equal(parameters.parseParameterValue('null'), null);
  assert.equal(parameters.parseParameterValue('""'), '');
});
await test('parameter editor: inherited defaults retain falsy values and are not rewritten', () => {
  assert.deepEqual(parameters.parameterState({ defaults: { top_p: 0 } }, '/top_p'), { mode: 'inherit', inherited: 0 });
  assert.deepEqual(parameters.parameterState({ defaults: { stop: null } }, '/stop'), { mode: 'inherit', inherited: null });
});
await test('parameter editor: budget edit writes only the declared wire field', () => {
  const original = { defaults: { max_tokens: 88 }, channel: { set: { max_tokens: 11, max_completion_tokens: 12, generationConfig: { maxOutputTokens: 13, vendor: false }, unknown: 0 }, extra: 'keep' }, models: [{ set: { max_tokens: 90 } }] };
  const next = parameters.patchGeneration(original, 'output_limit', { path: '/max_output_tokens' }, 'set', 512);
  assert.deepEqual(next.channel.set, { generationConfig: { vendor: false }, unknown: 0, max_output_tokens: 512 });
  assert.deepEqual(next.defaults, original.defaults);
  assert.deepEqual(next.models, original.models);
  assert.equal(next.channel.extra, 'keep');
});
await test('parameter editor: inherited budget clears legacy alias removal without touching other scopes', () => {
  const initial = { defaults: { max_completion_tokens: 64 }, channel: { set: { untouched: false }, remove: ['/max_completion_tokens', '/custom'] }, models: [{ remove: ['/max_tokens'] }] };
  const spec = { path: '/max_tokens' };
  assert.deepEqual(parameters.generationState(initial, 'output_limit', spec), { mode: 'remove', inherited: 64 });
  const set = parameters.patchGeneration(initial, 'output_limit', spec, 'set', 128);
  assert.deepEqual(set.channel.remove, ['/custom']);
  const inherited = parameters.patchGeneration(set, 'output_limit', spec, 'inherit');
  assert.deepEqual(inherited.channel.remove, ['/custom']);
  assert.deepEqual(inherited.channel.set, { untouched: false });
  assert.deepEqual(inherited.models, initial.models);
  assert.deepEqual(inherited.defaults, initial.defaults);
  assert.deepEqual(parameters.generationState(inherited, 'output_limit', spec), { mode: 'inherit', inherited: 64 });
});
await test('parameter editor: Gemini JSON pointer builds nested keys and preserves siblings', () => {
  const next = parameters.patchParameter({ channel: { set: { generationConfig: { topP: 0.5 } } } }, '/generationConfig/maxOutputTokens', 'set', 256);
  assert.deepEqual(next.channel.set, { generationConfig: { topP: 0.5, maxOutputTokens: 256 } });
  const inherited = parameters.patchParameter(next, '/generationConfig/maxOutputTokens', 'inherit');
  assert.deepEqual(inherited.channel.set, { generationConfig: { topP: 0.5 } });
});
await test('parameter editor: selecting a prompt override preserves an explicit empty string', () => {
  for (const mode of ['prepend', 'append', 'replace']) assert.deepEqual(parameters.patchSystemPrompt({}, mode, ''), { system_prompt: { mode, value: '' } });
  const current = { system_prompt: { mode: 'append', value: 'before', extension: false }, future: { keep: null } };
  const next = parameters.patchSystemPrompt(current, 'replace', '');
  assert.deepEqual(next.system_prompt, { mode: 'replace', value: '', extension: false });
  assert.deepEqual(next.future, { keep: null });
});
await test('parameter editor: saved sensitive header omits value; explicit empty is not inheritance', () => {
  const saved = { name: 'X-Api-Key', mode: 'set', secret: false, enabled: false, value: '********', extension: 0 };
  const keep = parameters.patchHeaderValue(saved, {});
  assert.equal(Object.hasOwn(keep, 'value'), false);
  assert.equal(keep.enabled, false);
  assert.equal(keep.extension, 0);
  const empty = parameters.patchHeaderValue(saved, {}, { edited: true, value: '' });
  assert.equal(empty.value, '');
  assert.equal(empty.mode, 'set');
  const inherit = parameters.patchHeaderValue(saved, { mode: 'inherit' });
  assert.equal(inherit.mode, 'inherit');
  assert.equal(Object.hasOwn(inherit, 'value'), false);
});
await test('parameter editor: authentication fields are concealed without an explicit secret flag', () => {
  const names = ['Authorization', 'Proxy-Authorization', 'Cookie', 'X-Api-Key', 'X-Auth-Token'];
  for (const name of names) assert.equal(parameters.sensitiveHeader(name, false), true);
  const view = parameters.policyForAdvanced({ headers: [{ name: 'Authorization', value: 'hidden', secret: false }, { name: 'X-Tag', value: '' }] });
  assert.equal(Object.hasOwn(view.headers[0], 'value'), false);
  assert.equal(view.headers[1].value, '');
});
await test('parameter editor: complete JSON round-trip retains unknown fields and scoped model order', () => {
  const policy = { future: { enabled: false }, defaults: { optional: null }, channel: { set: { x: 0 }, custom: [1, 2] }, models: [{ match: { requested_model: 'a' }, set: { x: '' } }, { match: { requested_model: 'a' }, remove: ['/x'] }], system_prompt: { mode: 'inherit', extension: 'keep' } };
  assert.deepEqual(parameters.applyAdvancedPolicy(JSON.stringify(policy), {}), policy);
});
await test('parameter editor: advanced secret omission retains session draft but explicit clear wins', () => {
  const current = { headers: [{ name: 'Authorization', mode: 'set', value: 'session-draft' }] };
  assert.equal(parameters.applyAdvancedPolicy('{"headers":[{"name":"Authorization","mode":"set"}]}', current).headers[0].value, 'session-draft');
  assert.equal(parameters.applyAdvancedPolicy('{"headers":[{"name":"Authorization","mode":"set","value":""}]}', current).headers[0].value, '');
  assert.equal(parameters.applyAdvancedPolicy('{"headers":[]}', current).headers.length, 0);
});
await test('parameter editor: duplicate header names cannot silently reuse the first secret', () => {
  assert.throws(() => parameters.assertUniqueHeaders([{ name: 'Authorization' }, { name: 'authorization', enabled: false }]), /重复/);
  assert.throws(() => parameters.applyAdvancedPolicy('{"headers":[{"name":"X-Tag"},{"name":"x-tag"}]}', {}), /重复/);
  const current = { headers: [{ name: 'Authorization', value: 'first' }, { name: 'authorization', value: 'second' }] };
  assert.throws(() => parameters.applyAdvancedPolicy('{"headers":[{"name":"Authorization","mode":"set"}]}', current), /无法安全保留/);
  assert.equal(parameters.applyAdvancedPolicy('{"headers":[{"name":"Authorization","mode":"set","value":"explicit"}]}', current).headers[0].value, 'explicit');
});
await test('parameter editor: unsafe pointers are rejected without prototype pollution', () => {
  assert.throws(() => parameters.patchParameter({}, '/__proto__/polluted', 'set', true), /不允许/);
  assert.equal({}.polluted, undefined);
  assert.throws(() => parameters.patchParameter({}, 'temperature', 'set', 0), /JSON Pointer/);
});
await test('parameter editor: generic preview is not incorrectly disabled by native-only marker', () => {
  const capabilities = { body_supported: true, native_preview: false, endpoints: ['/v1/chat/completions'] };
  assert.equal(channel.canPreview(capabilities, '/v1/chat/completions'), true);
  assert.equal(channel.canPreview(capabilities, '/v1/responses'), false);
  assert.equal(channel.canPreview({ ...capabilities, body_supported: false }, '/v1/chat/completions'), false);
  assert.equal(channel.canPreview({ ...capabilities, preview_supported: false }, '/v1/chat/completions'), false);
});
await test('parameter editor: unchanged redacted URL is omitted even when other channel fields change', () => {
  assert.equal(channel.shouldSendBaseURL({ provider: 'saved' }, 'https://example.test/v1', 'https://example.test/v1'), false);
  assert.equal(channel.shouldSendBaseURL({ provider: 'saved' }, 'https://new.test/v1', 'https://example.test/v1'), true);
  assert.equal(channel.shouldSendBaseURL(null, 'https://example.test/v1', ''), true);
});

await test('model editor: legacy model mappings round-trip without dropping multiple aliases', () => {
  const source = ['plain-model', { 'same-upstream': 'public-a' }, { 'same-upstream': 'public-b' }, 'same-upstream'];
  const rows = models.modelsToRows(source);
  assert.equal(rows.length, 4);
  assert.deepEqual(models.rowsToModels(rows), source);
});
await test('model editor: explicit identity aliases and mapping order are preserved', () => {
  const source = [{ alpha: 'alpha' }, { beta: 'b' }, 'gamma'];
  assert.deepEqual(models.rowsToModels(models.modelsToRows(source)), source);
  assert.deepEqual(models.modelsToRows([{ alpha: 'a', beta: 'b' }]), [{ upstream: 'alpha', alias: 'a' }, { upstream: 'beta', alias: 'b' }]);
});
await test('model editor: alias editing serializes upstream to public name, not the reverse', () => {
  const rows = models.modelsToRows(['upstream-a', { 'upstream-b': 'before' }]);
  rows[0].alias = ' public-a '; rows[1].alias = 'after';
  assert.deepEqual(models.rowsToModels(rows), [{ 'upstream-a': 'public-a' }, { 'upstream-b': 'after' }]);
  rows[1].alias = ''; rows[1].upstream = 'upstream-c';
  assert.deepEqual(models.rowsToModels(rows), [{ 'upstream-a': 'public-a' }, 'upstream-c']);
});
await test('model editor: selecting candidates never replaces existing aliases', () => {
  const rows = models.modelsToRows([{ upstream: 'a' }, { upstream: 'b' }, 'keep']);
  const selected = models.selectModelCandidates(rows, ['upstream', 'new', 'new'], true);
  assert.deepEqual(models.rowsToModels(selected), [{ upstream: 'a' }, { upstream: 'b' }, 'keep', 'new']);
  assert.equal(rows.length, 3, 'selection must not mutate its input array');
});
await test('model editor: deselection is scoped only to explicit filtered IDs', () => {
  const rows = models.modelsToRows(['first', { second: 'public-second' }, 'third']);
  assert.deepEqual(models.rowsToModels(models.selectModelCandidates(rows, ['first'], false)), [{ second: 'public-second' }, 'third']);
  assert.deepEqual(models.selectModelCandidates(rows, [], false), rows);
});
await test('model editor: discovery merges candidates but never selects or replaces rows', () => {
  const selected = models.modelsToRows([{ existing: 'kept' }]);
  const candidates = models.mergeModelCandidates([{ id: 'existing', name: 'Existing' }], [{ id: 'new', name: 'New model' }, { id: 'existing', name: 'Updated display name' }, { id: '' }, null]);
  assert.deepEqual(candidates, [{ id: 'existing', name: 'Updated display name' }, { id: 'new', name: 'New model' }]);
  assert.deepEqual(models.rowsToModels(selected), [{ existing: 'kept' }]);
});
await test('model editor: batch paste supports newlines, commas, spaces and localized separators', () => {
  assert.deepEqual(models.splitModelInput('alpha\nbeta, gamma\tdelta，epsilon；zeta;alpha'), ['alpha', 'beta', 'gamma', 'delta', 'epsilon', 'zeta']);
  assert.deepEqual(models.splitModelInput('  \n , '), []);
});
await test('model editor: invalid legacy entries are rejected rather than silently lost', () => {
  for (const invalid of [null, {}, ['okay', null], [{ upstream: ['a', 'b'] }], [{}]]) assert.throws(() => models.modelsToRows(invalid), /无法|不是|包含/);
});
await test('model editor: explicit models are required and invalid IDs are not serialized', () => {
  assert.throws(() => models.rowsToModels([{ upstream: 'all', alias: '' }]), /不能使用 all/);
  assert.throws(() => models.rowsToModels([{ upstream: '', alias: '' }]), /上游 ID/);
  assert.throws(() => models.rowsToModels([{ upstream: 'two words', alias: '' }]), /空白/);
  assert.deepEqual(models.rowsToModels([{ upstream: '__proto__', alias: 'public' }]), [{ ['__proto__']: 'public' }]);
});

await test('HTML escaping covers text and attribute injection', () => {
  assert.equal(ui.esc('<img src=x onerror="x">&\''), '&lt;img src=x onerror=&quot;x&quot;&gt;&amp;&#39;');
  assert.equal(ui.esc(null), '');
  assert.ok(!ui.field('name', '<script>', '" onfocus="evil').includes('value="" onfocus='));
  assert.ok(ui.jsonDetails({ unsafe: '</pre><script>x</script>' }).includes('&lt;/pre&gt;'));
});
await test('SVG renderer only uses fixed local paths', () => {
  const svg = icons.icon('<script>alert(1)</script>', '" onload="evil');
  assert.ok(svg.startsWith('<svg'));
  assert.ok(!svg.includes('onload'));
  assert.ok(!svg.includes('<script'));
  assert.ok(svg.includes('aria-hidden="true"'));
});
await test('unknown metric is not rendered as a fake zero', () => {
  for (const value of [null, undefined, NaN, '9']) assert.equal(ui.fmt(value), '—');
  assert.equal(ui.fmt(0), '0');
  assert.equal(ui.fmt(1234), '1,234');
});
await test('schema fields conceal stored secrets and preserve field defaults', () => {
  const html = ui.schemaFields([{ key: 'token', label: 'Token', type: 'string', secret: true, required: true }, { key: 'count', type: 'integer', default: 3 }], { token: 'stored-secret' });
  assert.ok(html.includes('type="password"'));
  assert.ok(!html.includes('stored-secret'));
  assert.ok(html.includes('value="3"'));
  assert.ok(html.includes('留空保留'));
});
await test('schema produces typed values and JSON objects', () => {
  const schema = [{ key: 'n', type: 'integer' }, { key: 'flag', type: 'boolean' }, { key: 'mode', type: 'string', enum: ['a', 'b'] }, { key: 'options', type: 'object' }];
  const result = ui.readSchema(form({ 'schema-0': { value: '7' }, 'schema-1': { value: 'on', checked: false }, 'schema-2': { value: '1' }, 'schema-3': { value: '{"a":1}' } }), schema);
  assert.deepEqual({ ...result }, { n: 7, flag: false, mode: 'b', options: { a: 1 } });
});
await test('unchanged secret is omitted for discovery/channel writes and retained only explicitly', () => {
  const schema = [{ key: 'token', type: 'string', secret: true, required: true }];
  const f = form({ 'schema-0': { value: '' } });
  assert.deepEqual({ ...ui.readSchema(f, schema, { token: '***' }) }, {});
  assert.deepEqual({ ...ui.readSchema(f, schema, { token: '***' }, 'schema', true) }, { token: '***' });
  assert.deepEqual({ ...ui.readSchema(form({ 'schema-0': { value: 'new' } }), schema, { token: '***' }) }, { token: 'new' });
});
await test('schema and advanced JSON reject invalid input', () => {
  assert.throws(() => ui.readSchema(form({ 'schema-0': { value: '2.4' } }), [{ key: 'n', type: 'integer' }]), /有效数字/);
  assert.throws(() => ui.readSchema(form({ 'schema-0': { value: '' } }), [{ key: 'name', type: 'string', required: true }]), /请填写/);
  assert.throws(() => ui.parseJSON('[]', '配置'), /JSON 对象/);
  assert.throws(() => ui.parseJSON('{}', '列表', 'array'), /JSON 数组/);
  assert.throws(() => ui.parseJSON('{', '配置'), /有效的 JSON/);
});
await test('API client uses same-origin cookie, in-memory CSRF, JSON and no-store', async () => {
  let call;
  globalThis.fetch = async (url, options) => { call = { url, options }; return new Response('{"ok":true}', { status: 200 }); };
  api.setSession({ csrf_token: 'csrf-fixture' });
  assert.deepEqual(await api.post('/channels', { revision: 'r1' }), { ok: true });
  assert.equal(call.url, '/admin/api/channels');
  assert.equal(call.options.credentials, 'same-origin');
  assert.equal(call.options.cache, 'no-store');
  assert.equal(call.options.headers['X-CSRF-Token'], 'csrf-fixture');
  assert.equal(call.options.headers['Content-Type'], 'application/json');
  assert.equal(call.options.body, '{"revision":"r1"}');
  await api.get('/channels');
  assert.equal(call.options.headers['X-CSRF-Token'], undefined);
});
await test('API multipart upload keeps browser boundary, cookie and CSRF without configuration options', async () => {
  let call;
  globalThis.fetch = async (url, options) => { call = { url, options }; return new Response('{"ok":true}', { status: 200 }); };
  api.setSession({ csrf_token: 'upload-csrf' });
  const file = new File([JSON.stringify(pluginPackage())], 'plugin.miora.json', { type: 'application/json' });
  await api.upload('/plugins/upload', file, { expected_revision: 7 });
  assert.equal(call.url, '/admin/api/plugins/upload');
  assert.equal(call.options.credentials, 'same-origin');
  assert.equal(call.options.headers['X-CSRF-Token'], 'upload-csrf');
  assert.equal(call.options.headers['Content-Type'], undefined);
  assert.equal(call.options.cache, 'no-store');
  assert.ok(call.options.body instanceof FormData);
  assert.deepEqual([...call.options.body.keys()], ['file', 'options']);
  assert.equal(call.options.body.get('file').name, file.name);
  assert.deepEqual(JSON.parse(call.options.body.get('options')), { expected_revision: 7 });
});
await test('API errors preserve backend details and explain revision conflict', async () => {
  globalThis.fetch = async () => new Response('{"error":{"code":"revision_conflict","message":"Changed"}}', { status: 409 });
  await assert.rejects(api.patch('/channels', {}), error => error.status === 409 && error.code === 'revision_conflict' && error.message.includes('刷新'));
  globalThis.fetch = async () => new Response('<html>Bad upstream</html>', { status: 502 });
  await assert.rejects(api.get('/usage'), error => error.code === 'invalid_response');
});
await test('expired session redirects login but failed login does not redirect-loop', async () => {
  let redirect;
  globalThis.window = { location: { replace: path => { redirect = path; } } };
  globalThis.fetch = async () => new Response('{"error":{"code":"unauthorized","message":"Session expired"}}', { status: 401 });
  await assert.rejects(api.get('/session'));
  assert.equal(redirect, '/login'); redirect = undefined;
  await assert.rejects(api.request('/login', { method: 'POST', body: { username: 'admin', password: 'fixture' }, redirect: false }));
  assert.equal(redirect, undefined);
});
await test('transport failure is understandable and has no credential-bearing dump', async () => {
  globalThis.fetch = async () => { throw new TypeError('failed to fetch private internal URL'); };
  await assert.rejects(api.get('/overview'), error => error.message === '无法连接到管理服务，请检查网络后重试。');
});

await test('selected model search keeps original row indices and distinct public aliases', () => {
  const rows = models.modelsToRows([{ shared: 'public-one' }, { shared: 'PUBLIC-two' }, 'unrelated']);
  const filtered = models.matchingModelRows(rows, '  public-TWO  ');
  assert.deepEqual(filtered, [{ row: rows[1], index: 1 }]);
  assert.equal(filtered[0].row, rows[1]);
  assert.deepEqual(models.matchingModelRows(rows, 'SHARED').map(item => item.index), [0, 1]);
  assert.equal(models.matchingModelRows(rows, 'missing').length, 0);
  assert.equal(models.matchingModelRows(rows, ' ').length, rows.length);
  assert.deepEqual(models.rowsToModels(rows), [{ shared: 'public-one' }, { shared: 'PUBLIC-two' }, 'unrelated']);
});

await test('batch model prices affect only the chosen rows and explicitly filled fields', () => {
  const original = { input: 2, models: { first: { input: 3, output: 8 }, second: { output: 9 }, unused: { input: 99 } } };
  const rows = models.modelsToRows([{ shared: 'first' }, { shared: 'second' }]);
  const pricing = models.createModelPricing(original);
  pricing.apply(models.matchingModelRows(rows, 'SECOND').map(item => item.row), { input: '', output: '0', cache_read: '.25' });
  assert.deepEqual(pricing.value(rows), { input: 2, models: { first: { input: 3, output: 8 }, second: { output: 0, cache_read: .25 }, unused: { input: 99 } } });
  assert.equal(original.models.second.output, 9);
  assert.equal(pricing.entry(rows[1]).values.input, 2);
});

await test('invalid or empty batch input does not partially mutate model pricing', () => {
  const rows = models.modelsToRows(['first', 'second']);
  const original = { models: { first: { input: 1 }, second: { input: 2 } } };
  const pricing = models.createModelPricing(original);
  for (const values of [{}, { input: ' ' }, { input: '4', output: 'oops' }, { cache_read: '-1' }, { output: 'Infinity' }, { cache_write: '1000001' }]) {
    assert.throws(() => pricing.apply(rows, values), /单价/);
    assert.deepEqual(pricing.value(rows), original);
  }
});

function modelsEditorFixture(options = {}) {
  const nodes = new Map(), listeners = new Map();
  function node(selector) {
    if (!nodes.has(selector)) nodes.set(selector, {
      value: '', innerHTML: '', textContent: '', hidden: false, disabled: false, dataset: {}, attributes: new Map(),
      matches() { return false; }, focus() {}, addEventListener() {},
      setAttribute(name, value) { this.attributes.set(name, value); }, removeAttribute(name) { this.attributes.delete(name); },
      classList: { add() {}, remove() {}, toggle() {} },
    });
    return nodes.get(selector);
  }
  const prices = models.PRICE_FIELDS.map(([key]) => ({ value: '', dataset: { batchPriceKey: key } }));
  const root = {
    innerHTML: '', querySelector: node,
    querySelectorAll(selector) { assert.equal(selector, '[data-batch-price-key]'); return prices; },
    addEventListener(type, listener) { listeners.set(type, listener); },
  };
  const editor = models.createModelsEditor(root, { ...options, discover: async () => ({ models: [], source: 'upstream' }) });
  return {
    editor, node, prices, root,
    search(value) { const input = node('[data-selected-search]'); input.value = value; listeners.get('input')({ target: input }); },
    key(target, changes = {}) {
      let prevented = false;
      listeners.get('keydown')({ target, key: 'Enter', ctrlKey: false, metaKey: false, isComposing: false,
        ...changes, preventDefault() { prevented = true; } });
      return prevented;
    },
    click(action, index) {
      const button = node(`[data-model-action="${action}"]`);
      button.dataset = { modelAction: action, rowIndex: String(index) };
      button.closest = () => button;
      listeners.get('click')({ target: button });
    },
  };
}

await test('model editor applies bulk prices to visible rows without changing hidden aliases', () => {
  let changed = 0, priced = 0;
  const fixture = modelsEditorFixture({ models: [{ shared: 'first' }, { shared: 'second' }, 'other'],
    pricing: { models: { first: { input: 1 }, second: { output: 9 } } },
    onChange() { changed++; }, onPricingChange() { priced++; } });
  const before = changed;
  fixture.search('second');
  assert.equal(changed, before);
  assert.equal(fixture.node('[data-selected-count]').textContent, '1 / 3');
  assert.match(fixture.node('.model-selected-list').innerHTML, /data-upstream-index="1"/);
  assert.doesNotMatch(fixture.node('.model-selected-list').innerHTML, /data-upstream-index="0"/);
  fixture.prices.find(input => input.dataset.batchPriceKey === 'input').value = '5';
  fixture.prices.find(input => input.dataset.batchPriceKey === 'output').value = '0';
  fixture.click('apply-prices');
  assert.deepEqual(fixture.editor.pricing(), { models: { first: { input: 1 }, second: { output: 0, input: 5 } } });
  assert.equal(priced, 1);
  assert.equal(changed, before);
  assert.match(fixture.node('[data-batch-price-message]').textContent, /已更新 1 项/);
  assert.match(fixture.root.innerHTML, /type="text" inputmode="decimal"/);
});

await test('filtered model removal uses the original row index and empty results cannot apply prices', () => {
  const fixture = modelsEditorFixture({ models: ['first', { shared: 'second' }, 'third'] });
  fixture.search('second');
  fixture.click('remove', 1);
  assert.deepEqual(fixture.editor.value(), ['first', 'third']);
  assert.equal(fixture.node('[data-model-action="apply-prices"]').disabled, true);
  assert.match(fixture.node('.model-selected-list').innerHTML, /没有匹配/);
  fixture.click('clear-selected-filter');
  assert.equal(fixture.node('[data-selected-search]').value, '');
  assert.equal(fixture.node('[data-model-action="apply-prices"]').disabled, false);
});

await test('Enter in model search does not submit, and Enter in bulk prices applies only the draft', () => {
  let priced = 0;
  const fixture = modelsEditorFixture({ models: ['first'], onPricingChange() { priced++; } });
  assert.equal(fixture.key(fixture.node('[data-selected-search]')), true);
  assert.equal(fixture.key(fixture.node('[data-model-search]')), true);
  assert.equal(fixture.key(fixture.node('[data-selected-search]'), { ctrlKey: true }), false);
  assert.equal(fixture.key(fixture.node('[data-selected-search]'), { isComposing: true }), false);
  const input = fixture.prices.find(input => input.dataset.batchPriceKey === 'input');
  input.value = '0'; input.matches = selector => selector === '[data-batch-price-key]';
  assert.equal(fixture.key(input), true);
  assert.deepEqual(fixture.editor.pricing(), { models: { first: { input: 0 } } });
  assert.equal(priced, 1);
  fixture.search('no-match');
  assert.equal(fixture.key(input), true);
  assert.equal(priced, 1);
});

await test('adding an alias from filtered models reveals the new row and carries its prices', () => {
  const fixture = modelsEditorFixture({ models: [{ shared: 'public-name' }], pricing: { models: { 'public-name': { input: 7 } } } });
  fixture.search('public-name');
  fixture.click('alias', 0);
  assert.equal(fixture.node('[data-selected-search]').value, '');
  assert.deepEqual(fixture.editor.snapshot(), [{ upstream: 'shared', alias: 'public-name' }, { upstream: 'shared', alias: '' }]);
  assert.deepEqual(fixture.editor.pricing().models.shared, { input: 7 });
  assert.match(fixture.node('.model-selected-list').innerHTML, /data-alias-index="1"/);
});

await test('save shortcut requires explicit Ctrl or Command and ignores composition and other modifiers', () => {
  const key = { key: 'Enter', ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, isComposing: false };
  assert.equal(ui.isSaveShortcut(key), false);
  assert.equal(ui.isSaveShortcut({ ...key, ctrlKey: true }), true);
  assert.equal(ui.isSaveShortcut({ ...key, metaKey: true }), true);
  for (const change of [{ isComposing: true }, { altKey: true }, { shiftKey: true }, { key: 'Escape' }]) {
    assert.equal(ui.isSaveShortcut({ ...key, ctrlKey: true, ...change }), false);
  }
});

await test('dialog shortcut uses the existing submit path and cannot duplicate an in-flight save', async () => {
  const listeners = new Map(), submit = { disabled: false }, errorBox = { hidden: true };
  let requests = 0, saves = 0, closed = 0, finish;
  const pending = new Promise(resolve => { finish = resolve; });
  const form = {
    addEventListener(type, listener) { listeners.set(type, listener); },
    querySelector(selector) { return selector === 'button[type="submit"]' ? submit : errorBox; },
    querySelectorAll(selector) { return selector === 'button' ? [submit] : []; },
    reportValidity() { return true; }, setAttribute() {}, removeAttribute() {},
    requestSubmit() { requests++; this.pending = listeners.get('submit')({ preventDefault() {} }); },
  };
  const modal = { open: false, classList: { toggle() {} }, showModal() { this.open = true; }, close() { this.open = false; closed++; } };
  const content = { innerHTML: '' };
  const document = { activeElement: null, querySelector(selector) { return selector === '#editor-dialog' ? modal : selector === '#dialog-content' ? content : form; } };
  const text = await source('ui.js');
  const factory = runInNewContext(text.slice(text.indexOf('export function dialog('), text.indexOf('export function confirmAction(')).replace(/^export /, '') + '\ndialog', {
    document, esc: ui.esc, icon: icons.icon, isSaveShortcut: ui.isSaveShortcut,
  });
  factory({ title: 'fixture', body: '', onSubmit: async () => { saves++; await pending; } });
  assert.equal(submit.title, 'Ctrl+Enter / Command+Enter');
  function press(changes) {
    let prevented = false;
    listeners.get('keydown')({ key: 'Enter', ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, isComposing: false,
      ...changes, preventDefault() { prevented = true; } });
    return prevented;
  }
  assert.equal(press({}), false);
  assert.equal(press({ ctrlKey: true, isComposing: true }), false);
  assert.equal(press({ metaKey: true }), true);
  assert.equal(saves, 1);
  assert.equal(press({ ctrlKey: true }), true);
  assert.equal(requests, 1);
  finish(); await form.pending;
  assert.equal(closed, 1);
  assert.equal(submit.disabled, false);
});

await test('channel filters restore URL search and type with escaped input markup', async () => {
  const text = await source('app.js');
  const query = 'Alpha " <img>';
  const params = new URLSearchParams({ q: query, type: 'gpt' });
  const render = runInNewContext(text.slice(text.indexOf('async function channels('), text.indexOf('async function editChannel(')) + '\nchannels', {
    read: async path => { assert.equal(path, '/channels'); return { channels: [{ engine: 'gpt' }], writable: true }; }, state: {},
    window: { location: { hash: `#channels?${params}` } }, URLSearchParams,
    head: () => '', button: () => '', icon: icons.icon, esc: ui.esc, fmt: ui.fmt, select: ui.select,
    table: () => '', channelRows: () => '', empty: ui.empty,
  });
  const html = await render();
  assert.ok(html.includes(`value="${ui.esc(query)}"`));
  assert.match(html, /value="gpt"\s+selected/);
  assert.match(html, /id="clear-channel-filters"/);
  assert.ok(!html.includes('<img>'));
});

await test('channel filtering updates counts and URL locally and clears both controls in one click', async () => {
  const controls = new Map();
  for (const selector of ['#channel-search', '#f-channel_type_filter', '#f-channel_group_filter', '#clear-channel-filters', '#search-empty', '#channel-filter-count']) {
    controls.set(selector, { value: '', hidden: false, disabled: false, textContent: '', listeners: new Map(),
      addEventListener(type, listener) { this.listeners.set(type, listener); }, focus() { this.focused = true; } });
  }
  const search = controls.get('#channel-search'), engine = controls.get('#f-channel_type_filter');
  search.value = 'ALPHA'; engine.value = 'gpt';
  const rows = [
    { dataset: { search: 'alpha one', engine: 'gpt' } },
    { dataset: { search: 'alpha two', engine: 'claude' } },
    { dataset: { search: 'beta three', engine: 'gpt' } },
  ];
  const window = { location: { hash: '#channels' }, history: { replaceState(state, title, url) { window.location.hash = url; } } };
  const text = await source('app.js');
  const bind = runInNewContext(text.slice(text.indexOf('function bindPage('), text.indexOf("main.addEventListener('click'")) + '\nbindPage', {
    main: { querySelector: selector => controls.get(selector), querySelectorAll: () => rows },
    document: { querySelector: () => null }, state: { channels: { channels: rows } }, window, URLSearchParams, fmt: ui.fmt,
  });
  bind();
  assert.deepEqual(rows.map(row => row.hidden), [false, true, true]);
  assert.equal(controls.get('#channel-filter-count').textContent, '显示 1 / 3 个渠道');
  assert.equal(new URLSearchParams(window.location.hash.split('?')[1]).get('q'), 'ALPHA');
  assert.equal(new URLSearchParams(window.location.hash.split('?')[1]).get('type'), 'gpt');
  search.value = 'no-match'; search.listeners.get('input')();
  assert.equal(controls.get('#search-empty').hidden, false);
  controls.get('#clear-channel-filters').listeners.get('click')();
  assert.deepEqual(rows.map(row => row.hidden), [false, false, false]);
  assert.equal(controls.get('#channel-filter-count').textContent, '共 3 个渠道');
  assert.equal(engine.value, '');
  assert.equal(window.location.hash, '#channels');
  assert.equal(search.focused, true);
  assert.equal(controls.get('#clear-channel-filters').disabled, true);
});
