import * as api from './api.js';
import { icon, aiIcon, modelBrand } from './icons.js';
import { esc, fmt, field, select, badge, dialog, failure, empty, parseJSON, toast } from './ui.js';

export function buildTestExtra({ temperature = '', topP = '', reasoningEffort = '' } = {}, baseExtra = {}) {
  const extra = { ...baseExtra };
  if (temperature !== '' && temperature !== null && temperature !== undefined) {
    const val = Number(temperature);
    if (Number.isFinite(val)) extra.temperature = val;
  }
  if (topP !== '' && topP !== null && topP !== undefined) {
    const val = Number(topP);
    if (Number.isFinite(val)) extra.top_p = val;
  }
  if (reasoningEffort) extra.reasoning_effort = reasoningEffort;
  return extra;
}

export function testPayload(endpoint, model, prompt, limit, extra = {}) {
  const request = { ...extra, model, stream: false };
  if (endpoint === '/v1/responses') { request.input = prompt; if (limit !== '') request.max_output_tokens = Number(limit); }
  else { request.messages = [{ role: 'user', content: prompt }]; if (limit !== '') request.max_tokens = Number(limit); }
  return request;
}
export function testOutput(response) {
  if (!response || typeof response !== 'object') return '';
  if (response.choices) return response.choices.map(choice => choice.message?.content || choice.message?.reasoning_content || '').join('\n');
  if (typeof response.text === 'string') return response.text;
  return (response.output || []).flatMap(item => item.content || []).filter(part => part.type === 'output_text' || part.type === 'text').map(part => part.text || '').join('\n');
}
export function testErrorText(response) {
  if (!response || typeof response !== 'object') return '请求失败';
  return response.error?.message || response.message || response.detail || response.text || JSON.stringify(response);
}
export function testUsage(response) {
  const usage = response?.usage || {};
  return { input: usage.input_tokens ?? usage.prompt_tokens, output: usage.output_tokens ?? usage.completion_tokens,
    cached: usage.input_tokens_details?.cached_tokens ?? usage.prompt_tokens_details?.cached_tokens };
}

export function availableTestModels(rows, provider = '', targeted = false) {
  return [...new Set(rows.filter(row => !provider || targeted && row.provider === provider).map(row => row.model))];
}

export function openChannelTestDialog(channelOrList, options = {}) {
  const channelList = Array.isArray(channelOrList) ? channelOrList : [channelOrList].filter(Boolean);
  const multiChannel = channelList.length > 1;
  const writable = options.writable !== false;
  const items = [];
  for (const ch of channelList) {
    const rawModels = Array.isArray(ch?.models) ? ch.models : [];
    const seen = new Set();
    for (const m of rawModels) {
      const model = typeof m === 'string' ? m : Object.values(m || {})[0];
      if (!model || seen.has(model)) continue;
      seen.add(model);
      items.push({ key: `${ch.provider}::${model}`, provider: ch.provider, model });
    }
  }
  const selectedKeys = new Set(items.map(item => item.key));
  const results = new Map();
  let form, closed = false, running = false, batchController = null;
  function readParams() {
    if (!form) return { endpoint: '/v1/chat/completions', prompt: 'hi', limit: '16', extra: {}, concurrency: 3 };
    const endpoint = form.querySelector('[data-test-param="endpoint"]')?.value || '/v1/chat/completions';
    const prompt = form.querySelector('[data-test-param="prompt"]')?.value || 'hi';
    const limit = form.querySelector('[data-test-param="max_tokens"]')?.value ?? '16';
    const tempSelect = form.querySelector('[data-test-param="temperature"]')?.value ?? '';
    const tempCustom = form.querySelector('[data-test-param="temperature-custom"]')?.value ?? '';
    const temperature = tempSelect === 'custom' ? tempCustom : tempSelect;
    const topP = form.querySelector('[data-test-param="top_p"]')?.value ?? '';
    const reasoningEffort = form.querySelector('[data-test-param="reasoning_effort"]')?.value ?? '';
    const concurrency = Math.max(1, Number(form.querySelector('[data-test-param="concurrency"]')?.value || 3));
    return { endpoint, prompt, limit, extra: buildTestExtra({ temperature, topP, reasoningEffort }), concurrency };
  }
  function visibleItems() {
    if (!form) return items;
    const query = (form.querySelector('[data-channel-test-search]')?.value || '').trim().toLowerCase();
    const statusFilter = form.querySelector('[data-channel-test-filter]')?.value || '';
    return items.filter(item => {
      if (query && !`${item.provider} ${item.model}`.toLowerCase().includes(query)) return false;
      if (!statusFilter) return true;
      const st = results.get(item.key)?.status || 'pending';
      return st === statusFilter;
    });
  }
  function renderRows() {
    if (!form || closed) return;
    const visible = visibleItems();
    const selectedVisible = visible.filter(item => selectedKeys.has(item.key));
    const listRoot = form.querySelector('[data-channel-test-list]');
    const allBtn = form.querySelector('[data-channel-test-all]');
    const retryBtn = form.querySelector('[data-channel-test-retry-failed]');
    const removeBtn = form.querySelector('[data-channel-test-remove-failed]');
    const stopBtn = form.querySelector('[data-channel-test-stop]');
    const failedCount = items.filter(item => results.get(item.key)?.status === 'error').length;
    const okCount = items.filter(item => results.get(item.key)?.status === 'ok').length;
    if (allBtn) {
      allBtn.disabled = running || !selectedVisible.length;
      allBtn.innerHTML = `${icon('pulse')}批量测试 (${selectedVisible.length})`;
    }
    if (retryBtn) {
      retryBtn.hidden = failedCount === 0;
      retryBtn.disabled = running;
      retryBtn.innerHTML = `${icon('refresh')}重测失败 (${failedCount})`;
    }
    if (removeBtn) {
      removeBtn.hidden = !writable || failedCount === 0;
      removeBtn.disabled = running;
      removeBtn.innerHTML = `${icon('trash')}移除失败 (${failedCount})`;
    }
    if (stopBtn) stopBtn.hidden = !running;
    const summaryEl = form.querySelector('[data-channel-test-summary]');
    if (summaryEl) {
      summaryEl.innerHTML = (okCount || failedCount)
        ? `${badge(`成功 ${okCount}`, 'green')} ${failedCount ? badge(`失败 ${failedCount}`, 'orange') : ''}`
        : `<span class="hint">已选 ${selectedVisible.length} / ${visible.length} 个模型</span>`;
    }
    if (!items.length) {
      listRoot.innerHTML = empty('暂未配置模型', '请先在「编辑 -> 模型与映射」中获取或添加模型。', 'box');
      return;
    }
    if (!visible.length) {
      listRoot.innerHTML = `<p class="hint">没有匹配的模型。</p>`;
      return;
    }
    const allChecked = visible.length > 0 && visible.every(item => selectedKeys.has(item.key));
    listRoot.innerHTML = `<div class="table-scroll channel-test-scroll"><table class="channel-test-table"><thead><tr><th class="test-col-check"><input type="checkbox" data-channel-test-check-all aria-label="全选或取消全选"${allChecked ? ' checked' : ''}></th>${multiChannel ? '<th>渠道</th>' : ''}<th>模型</th><th>状态与响应</th><th>操作</th></tr></thead><tbody>${visible.map(item => {
      const state = results.get(item.key);
      const checked = selectedKeys.has(item.key);
      let statusHtml = `<span class="hint">未测试</span>`;
      if (state?.status === 'testing') {
        statusHtml = `${badge('测试中…', 'purple')}`;
      } else if (state?.status === 'ok') {
        const reply = (state.reply || '').trim();
        statusHtml = `${badge(`${fmt(state.elapsed_ms)} ms`, 'green')}${reply ? ` <span class="subline">${esc(reply.slice(0, 100))}</span>` : ''}`;
      } else if (state?.status === 'error') {
        statusHtml = `${badge(`HTTP ${state.http_status || 'ERR'}`, 'orange')}<span class="subline text-danger">${esc((state.error || '请求失败').slice(0, 140))}</span>`;
      }
      return `<tr class="channel-test-row"><td class="test-col-check"><input type="checkbox" data-channel-test-check="${esc(item.key)}" aria-label="选择 ${esc(item.model)}"${checked ? ' checked' : ''}></td>${multiChannel ? `<td class="test-col-provider">${badge(item.provider)}</td>` : ''}<td class="test-col-model"><div class="table-name"><span class="row-icon">${aiIcon(modelBrand(item.model))}</span><strong>${esc(item.model)}</strong></div></td><td class="test-col-status">${statusHtml}</td><td class="test-col-action"><button type="button" class="button small" data-channel-test-single="${esc(item.key)}"${state?.status === 'testing' ? ' disabled' : ''}>${icon('pulse')}测试</button></td></tr>`;
    }).join('')}</tbody></table></div>`;
  }
  async function removeFailedModels() {
    const failedItems = items.filter(item => results.get(item.key)?.status === 'error');
    if (!failedItems.length || running) return;
    const byProvider = new Map();
    for (const item of failedItems) {
      if (!byProvider.has(item.provider)) byProvider.set(item.provider, new Set());
      byProvider.get(item.provider).add(item.model);
    }
    let removedTotal = 0;
    for (const [provider, failedSet] of byProvider.entries()) {
      const detail = await api.get(`/channels/detail?${new URLSearchParams({ provider })}`);
      const rawModels = Array.isArray(detail.base?.model) ? detail.base.model : [];
      const nextModels = rawModels.filter(entry => {
        if (typeof entry === 'string') return !failedSet.has(entry);
        if (entry && typeof entry === 'object') {
          const [up, pub] = Object.entries(entry)[0] || [];
          return !failedSet.has(pub) && !failedSet.has(up);
        }
        return true;
      });
      if (!nextModels.length) {
        toast(`渠道「${provider}」至少需保留 1 个模型，未清空`);
        continue;
      }
      await api.patch('/channels', { revision: detail.revision, provider, patch: { model: nextModels } });
      for (let i = items.length - 1; i >= 0; i--) {
        if (items[i].provider === provider && failedSet.has(items[i].model)) {
          selectedKeys.delete(items[i].key);
          results.delete(items[i].key);
          items.splice(i, 1);
          removedTotal++;
        }
      }
    }
    if (removedTotal > 0) {
      toast(`已移除 ${removedTotal} 个测试失败模型`);
      renderRows();
      await options.onUpdated?.();
    }
  }
  async function testSingle(item, signal) {
    const { endpoint, prompt, limit, extra } = readParams();
    results.set(item.key, { status: 'testing' });
    renderRows();
    try {
      const res = await api.request('/test', {
        method: 'POST',
        signal,
        timeoutMs: 65000,
        body: { direct: true, provider: item.provider, model: item.model, endpoint, request: testPayload(endpoint, item.model, prompt, limit, extra) },
      });
      if (closed) return;
      if (res.ok) {
        results.set(item.key, { status: 'ok', http_status: res.http_status, elapsed_ms: res.elapsed_ms, reply: testOutput(res.response) });
      } else {
        results.set(item.key, { status: 'error', http_status: res.http_status, elapsed_ms: res.elapsed_ms, error: testErrorText(res.response) });
      }
    } catch (err) {
      if (signal?.aborted || closed) return;
      results.set(item.key, { status: 'error', http_status: 'ERR', elapsed_ms: 0, error: err.message });
    }
    renderRows();
  }
  async function runQueue(queue) {
    if (running || !queue.length) return;
    running = true;
    batchController = new AbortController();
    const { concurrency } = readParams();
    const progressEl = form.querySelector('[data-channel-test-progress]');
    let completed = 0, cursor = 0;
    const total = queue.length;
    if (progressEl) progressEl.textContent = `0 / ${total} 正在并发测试…`;
    renderRows();
    const workers = Array.from({ length: Math.min(concurrency, total) }, async () => {
      while (!batchController.signal.aborted && !closed) {
        const idx = cursor++;
        if (idx >= total) break;
        await testSingle(queue[idx], batchController.signal);
        completed++;
        if (progressEl && !closed) progressEl.textContent = `${completed} / ${total} 已测`;
      }
    });
    try {
      await Promise.all(workers);
    } finally {
      running = false;
      if (!closed) {
        if (progressEl) progressEl.textContent = `${batchController.signal.aborted ? '已停止' : '全部完成'} · ${completed} / ${total}`;
        renderRows();
      }
    }
  }
  const singleProvider = !multiChannel ? channelList[0]?.provider || '' : '';
  form = dialog({
    title: multiChannel ? `批量模型测活（${channelList.length} 个渠道 · ${items.length} 个模型）` : `${singleProvider} · 模型测活`,
    size: 'wide',
    submitLabel: '关闭',
    showCancel: false,
    body: `<div class="toolbar channel-test-toolbar"><div class="inline-row channel-test-filters"><div class="search-field">${icon('search')}<input type="search" data-channel-test-search placeholder="搜索模型…" aria-label="搜索模型"></div><select data-channel-test-filter aria-label="状态筛选"><option value="">全部状态</option><option value="ok">仅看成功</option><option value="error">仅看失败</option><option value="pending">未测试</option></select><span data-channel-test-summary></span></div><div class="table-actions"><span class="hint" data-channel-test-progress role="status"></span><button type="button" class="button small" data-channel-test-invert>反选</button><button type="button" class="button small" data-channel-test-retry-failed hidden>${icon('refresh')}重测失败</button><button type="button" class="button small text-danger" data-channel-test-remove-failed hidden>${icon('trash')}移除失败</button><button type="button" class="button small primary" data-channel-test-all>${icon('pulse')}批量测试</button><button type="button" class="button small text-danger" data-channel-test-stop hidden>停止</button></div></div><details class="test-params-details"><summary>测试参数设置（默认发 "hi" · 3 并发 · 可自选温度与思考强度）</summary><div class="test-params-bar"><label class="test-param-item"><span>测试内容</span><input type="text" data-test-param="prompt" value="hi" placeholder="例如 hi"></label><label class="test-param-item"><span>温度 (Temp)</span><div class="inline-row"><select data-test-param="temperature"><option value="">默认 (不传)</option><option value="0">0 (确定)</option><option value="0.2">0.2 (严谨)</option><option value="0.5">0.5 (均衡)</option><option value="0.7">0.7 (标准)</option><option value="1">1.0 (发散)</option><option value="custom">自定义…</option></select><input type="number" data-test-param="temperature-custom" step="0.1" min="0" max="2" placeholder="0.0-2.0" hidden></div></label><label class="test-param-item"><span>Top P</span><input type="number" data-test-param="top_p" step="0.05" min="0" max="1" placeholder="默认不传"></label><label class="test-param-item"><span>最大 Token</span><input type="number" data-test-param="max_tokens" step="1" min="1" value="16" placeholder="16"></label><label class="test-param-item"><span>思考强度</span><select data-test-param="reasoning_effort"><option value="">默认 (不传)</option><option value="low">low</option><option value="medium">medium</option><option value="high">high</option></select></label><label class="test-param-item"><span>并发数</span><select data-test-param="concurrency"><option value="1">1 并发</option><option value="3" selected>3 并发</option><option value="5">5 并发</option><option value="10">10 并发</option></select></label><label class="test-param-item"><span>端点</span><select data-test-param="endpoint"><option value="/v1/chat/completions">Chat</option><option value="/v1/responses">Responses</option></select></label></div></details><div data-channel-test-list></div>`,
    onSubmit: async () => {},
    onClose: () => { closed = true; batchController?.abort(); },
    onMount: mounted => {
      form = mounted;
      renderRows();
      form.querySelector('[data-channel-test-search]')?.addEventListener('input', renderRows);
      form.querySelector('[data-channel-test-filter]')?.addEventListener('change', renderRows);
      const tempSelect = form.querySelector('[data-test-param="temperature"]');
      const tempCustom = form.querySelector('[data-test-param="temperature-custom"]');
      tempSelect?.addEventListener('change', () => {
        if (tempCustom) {
          tempCustom.hidden = tempSelect.value !== 'custom';
          if (!tempCustom.hidden) tempCustom.focus();
        }
      });
      form.addEventListener('change', event => {
        if (event.target.matches('[data-channel-test-check-all]')) {
          const checked = event.target.checked;
          for (const item of visibleItems()) {
            if (checked) selectedKeys.add(item.key);
            else selectedKeys.delete(item.key);
          }
          renderRows();
          return;
        }
        if (event.target.matches('[data-channel-test-check]')) {
          const key = event.target.dataset.channelTestCheck;
          if (event.target.checked) selectedKeys.add(key);
          else selectedKeys.delete(key);
          renderRows();
        }
      });
      form.addEventListener('click', event => {
        const single = event.target.closest('[data-channel-test-single]');
        if (single) {
          const item = items.find(i => i.key === single.dataset.channelTestSingle);
          if (item) testSingle(item, batchController?.signal);
          return;
        }
        if (event.target.closest('[data-channel-test-invert]')) {
          for (const item of visibleItems()) {
            if (selectedKeys.has(item.key)) selectedKeys.delete(item.key);
            else selectedKeys.add(item.key);
          }
          renderRows();
          return;
        }
        if (event.target.closest('[data-channel-test-all]')) {
          runQueue(visibleItems().filter(item => selectedKeys.has(item.key)));
        }
        if (event.target.closest('[data-channel-test-retry-failed]')) {
          runQueue(items.filter(i => results.get(i.key)?.status === 'error'));
        }
        if (event.target.closest('[data-channel-test-remove-failed]')) {
          removeFailedModels().catch(err => toast(err.message));
        }
        if (event.target.closest('[data-channel-test-stop]')) batchController?.abort();
      });
    },
  });
}

export async function mountTesting(root, { signal }) {
  const keys = (await api.get('/api-keys', { signal })).api_keys.filter(key => key.enabled);
  if (signal.aborted) return;
  const seed = new URLSearchParams(window.location.hash.split('?')[1] || '');
  const keyOptions = [{ value: 0, label: '直连渠道测试（无需 API Key）' }, ...keys.map(key => ({ value: key.position, label: `${key.name || `Key ${key.position}`} · ${key.groups.join(', ')} · ${key.role}` }))];
  const selectedPosition = seed.get('provider') ? 0 : (keys.find(key => key.position === Number(seed.get('position')))?.position ?? (keys[0]?.position ?? 0));
  root.innerHTML = `<div class="page-heading"><h1>模型测试</h1><a class="button" href="#logs">${icon('file')}请求日志</a></div><div class="test-layout"><section class="card test-controls"><form id="model-test-form">${select('position', '测试方式 / 密钥', keyOptions, selectedPosition)}${select('endpoint', 'API 端点', [{ value: '/v1/chat/completions', label: 'Chat Completions' }, { value: '/v1/responses', label: 'Responses' }], '/v1/chat/completions')}${select('provider', '目标渠道', [{ value: '', label: '全部可用渠道' }])}<p id="test-target-hint" class="hint"></p>${select('model', '模型', [{ value: '', label: '正在读取可用模型' }])}${field('prompt', '输入提示词', 'hi', { type: 'textarea', required: true, rows: 3 })}<div class="field-row">${field('temperature', '温度 (Temperature)', '', { type: 'number', min: 0, max: 2, step: 'any', placeholder: '默认不传，如 0.7' })}${field('top_p', 'Top P', '', { type: 'number', min: 0, max: 1, step: 'any', placeholder: '默认不传，如 0.95' })}</div><div class="field-row">${field('limit', '最大输出 Token', '16', { type: 'number', min: 1, step: 1, placeholder: '留空使用默认' })}${select('reasoning_effort', '思考强度', [{ value: '', label: '默认（不传）' }, { value: 'low', label: 'low' }, { value: 'medium', label: 'medium' }, { value: 'high', label: 'high' }], '')}</div>${select('concurrency', '批量测试并发数', [{ value: '1', label: '1 并发（串行）' }, { value: '3', label: '3 并发' }, { value: '5', label: '5 并发' }, { value: '10', label: '10 并发' }], '3')}<details class="advanced"><summary>额外 JSON 参数</summary>${field('extra', 'JSON 参数', '{}', { type: 'textarea', rows: 3, help: '附加自定义字段。' })}</details><div class="page-actions"><button class="button primary" id="test-run" type="submit">${icon('pulse')}测试当前模型</button><button class="button" id="test-batch" type="button">批量测试全部模型</button><button class="button text-danger" id="test-stop" type="button" hidden>停止</button></div><div id="test-error" role="alert"></div></form></section><section class="card test-results"><div class="card-header"><h2>测试结果</h2><span class="hint" id="test-progress" role="status"></span></div><div id="test-results">${empty('尚未运行测试', '', 'pulse')}</div></section></div>`;
  const form = root.querySelector('#model-test-form');
  const error = root.querySelector('#test-error');
  const progress = root.querySelector('#test-progress');
  const resultsView = root.querySelector('#test-results');
  let rows = [], targeted = false, running = false, batchController, modelsController, sequence = 0, firstLoad = true;
  function updateModels() {
    const previous = form.elements.model.value;
    const provider = form.elements.provider.value;
    const models = availableTestModels(rows, provider, targeted);
    root.querySelector('#test-target-hint').textContent = provider ? `当前仅测试渠道「${provider}」上的模型。` : '批量测试将覆盖当前筛选下的全部模型。';
    form.elements.model.innerHTML = models.length ? models.map(model => `<option value="${esc(model)}">${esc(model)}</option>`).join('') : '<option value="">暂无可用模型</option>';
    if (models.includes(previous)) form.elements.model.value = previous;
    root.querySelector('#test-run').disabled = !models.length || running;
    root.querySelector('#test-batch').disabled = !models.length || running;
    root.querySelector('#test-batch').textContent = `批量测试全部 (${models.length})`;
  }
  async function loadModels() {
    modelsController?.abort(); modelsController = new AbortController(); const token = ++sequence;
    form.elements.model.innerHTML = '<option value="">正在读取可用模型</option>';
    root.querySelector('#test-run').disabled = true; root.querySelector('#test-batch').disabled = true;
    error.textContent = '';
    try {
      const data = await api.get(`/test/models?${new URLSearchParams({ position: form.elements.position.value, endpoint: form.elements.endpoint.value })}`, { signal: modelsController.signal });
      if (signal.aborted || token !== sequence) return;
      rows = data.models; targeted = data.targeted;
      const selected = firstLoad ? seed.get('provider') || '' : form.elements.provider.value;
      firstLoad = false;
      const available = targeted && rows.some(row => row.provider === selected);
      form.elements.provider.innerHTML = '<option value="">全部可用渠道</option>' + (targeted ? [...new Set(rows.map(row => row.provider))].map(provider => `<option value="${esc(provider)}">${esc(provider)}</option>`).join('') : '')
        + (selected && !available ? `<option value="${esc(selected)}" disabled>目标不可用 · ${esc(selected)}</option>` : '');
      form.elements.provider.value = selected;
      updateModels();
    } catch (cause) { if (!signal.aborted && token === sequence && !modelsController.signal.aborted) error.innerHTML = failure(cause); }
  }
  function resultRow(label, result) {
    const usage = testUsage(result.response);
    const text = testOutput(result.response);
    return `<article class="test-result"><div class="test-result-heading"><strong>${esc(label)}</strong><span class="badge ${result.ok ? 'green' : 'orange'}">HTTP ${esc(result.http_status)}</span><span class="hint">${fmt(result.elapsed_ms)} ms</span>${result.request_id ? `<a class="button small" href="#logs?request_id=${encodeURIComponent(result.request_id)}">查看日志</a>` : ''}</div><div class="test-token-row"><span>输入 ${usage.input === undefined ? '—' : fmt(usage.input)}</span><span>输出 ${usage.output === undefined ? '—' : fmt(usage.output)}</span><span>缓存命中 ${usage.cached === undefined ? '—' : fmt(usage.cached)}</span></div>${text ? `<pre class="test-output">${esc(text)}</pre>` : ''}<details ${result.ok ? '' : 'open'}><summary>响应 / 用量明细</summary><pre>${esc(JSON.stringify(result.response, null, 2))}</pre></details></article>`;
  }
  async function run(batch) {
    if (running || !form.reportValidity()) return;
    error.textContent = '';
    let baseExtra;
    try { baseExtra = parseJSON(form.elements.extra.value || '{}', '额外请求参数'); }
    catch (cause) { error.innerHTML = failure(cause); return; }
    const extra = buildTestExtra({
      temperature: form.elements.temperature.value,
      topP: form.elements.top_p.value,
      reasoningEffort: form.elements.reasoning_effort.value,
    }, baseExtra);
    const position = Number(form.elements.position.value), endpoint = form.elements.endpoint.value, provider = form.elements.provider.value;
    const concurrency = Math.max(1, Number(form.elements.concurrency?.value || 3));
    const queue = batch
      ? (position === 0 && !provider
        ? rows.map(r => ({ provider: r.provider, model: r.model, label: `${r.provider} / ${r.model}` }))
        : availableTestModels(rows, provider, targeted).map(m => ({ provider: targeted ? provider : '', model: m, label: provider ? `${provider} / ${m}` : m })))
      : [{ provider: targeted ? (provider || rows.find(r => r.model === form.elements.model.value)?.provider || '') : '', model: form.elements.model.value, label: form.elements.model.value }];
    if (!queue.length || !queue[0].model) return;
    const prompt = form.elements.prompt.value, limit = form.elements.limit.value;
    running = true; batchController = new AbortController();
    form.querySelectorAll('input, textarea, select, button').forEach(control => { control.disabled = true; });
    const stop = root.querySelector('#test-stop'); stop.hidden = false; stop.disabled = false;
    resultsView.textContent = ''; let completed = 0, cursor = 0;
    try {
      const workers = Array.from({ length: batch ? Math.min(concurrency, queue.length) : 1 }, async () => {
        while (!batchController.signal.aborted) {
          const idx = cursor++;
          if (idx >= queue.length) break;
          const item = queue[idx];
          progress.textContent = `${completed} / ${queue.length} · 正在测试 ${item.label}`;
          try {
            const result = await api.request('/test', { method: 'POST', signal: batchController.signal, timeoutMs: 185000,
              body: { position, direct: position === 0, endpoint, provider: item.provider, model: item.model, request: testPayload(endpoint, item.model, prompt, limit, extra) } });
            if (signal.aborted) return;
            resultsView.insertAdjacentHTML('beforeend', resultRow(item.label, result));
          } catch (cause) {
            if (batchController.signal.aborted) break;
            resultsView.insertAdjacentHTML('beforeend', `<article class="test-result"><strong>${esc(item.label)}</strong>${failure(cause)}</article>`);
          }
          completed++;
          if (!signal.aborted) progress.textContent = `${completed} / ${queue.length} 已完成`;
        }
      });
      await Promise.all(workers);
      if (!signal.aborted) progress.textContent = `${batchController.signal.aborted ? '已停止' : '已完成'} · ${completed} / ${queue.length}`;
    } finally {
      running = false;
      if (!signal.aborted) { form.querySelectorAll('input, textarea, select, button').forEach(control => { control.disabled = false; }); stop.hidden = true; updateModels(); }
    }
  }
  form.addEventListener('submit', event => { event.preventDefault(); run(false); }, { signal });
  root.querySelector('#test-batch').addEventListener('click', () => run(true), { signal });
  root.querySelector('#test-stop').addEventListener('click', () => batchController?.abort(), { signal });
  form.elements.position.addEventListener('change', loadModels, { signal });
  form.elements.endpoint.addEventListener('change', loadModels, { signal });
  form.elements.provider.addEventListener('change', updateModels, { signal });
  signal.addEventListener('abort', () => { modelsController?.abort(); batchController?.abort(); }, { once: true });
  await loadModels();
}
