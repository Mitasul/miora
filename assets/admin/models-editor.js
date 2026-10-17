import { icon, aiIcon, modelBrand } from './icons.js';
import { esc, fmt, badge } from './ui.js';

export const PRICE_FIELDS = [['input', '普通输入'], ['output', '输出'], ['cache_read', '缓存读取'], ['cache_write', '缓存写入（5 分钟）'], ['cache_write_1h', '缓存写入（1 小时）']];
export function readUsagePricing(values, original = {}) {
  const prices = { ...original };
  for (const [key] of PRICE_FIELDS) {
    const raw = String(values[key] ?? '').trim();
    if (raw === '') { if (Object.hasOwn(original, key)) prices[key] = null; continue; }
    const price = Number(raw);
    if (!Number.isFinite(price) || price < 0 || price > 1000000) throw new Error('单价必须为 0–1000000 的数字，留空表示未配置。');
    prices[key] = price;
  }
  return Object.keys(prices).length ? prices : undefined;
}
export function createModelPricing(original = {}) {
  const entries = new WeakMap(), originalNames = new Set();
  const catalog = new Map(Object.entries(original.models || {}));
  const defaults = Object.fromEntries(PRICE_FIELDS.filter(([key]) => Object.hasOwn(original, key)).map(([key]) => [key, original[key]]));
  function entry(row) {
    if (!entries.has(row)) {
      const name = row.alias.trim() || row.upstream.trim();
      const specific = catalog.get(name) || {};
      const effective = { ...defaults, ...specific };
      originalNames.add(name);
      entries.set(row, { specific, values: Object.fromEntries(PRICE_FIELDS.map(([key]) => [key, effective[key] ?? ''])), expanded: false });
    }
    return entries.get(row);
  }
  return {
    entry,
    copy(source, target) {
      entries.set(target, { specific: {}, values: { ...entry(source).values }, expanded: false });
    },
    apply(rows, values) {
      const prices = readUsagePricing(values);
      if (!prices) throw new Error('请至少填写一项单价。');
      for (const row of rows) Object.assign(entry(row).values, prices);
    },
    value(rows) {
      rows.forEach(entry);
      const models = new Map(catalog);
      for (const name of originalNames) models.delete(name);
      for (const row of rows) {
        const state = entry(row), name = row.alias.trim() || row.upstream.trim();
        models.delete(name);
        let rates;
        try { rates = readUsagePricing(state.values, { ...defaults, ...state.specific }) || {}; }
        catch (error) { throw new Error(`${name}：${error.message}`); }
        for (const [key] of PRICE_FIELDS) if (!Object.hasOwn(state.specific, key) && rates[key] === defaults[key]) delete rates[key];
        if (Object.keys(rates).length) models.set(name, rates);
      }
      const prices = { ...original };
      if (models.size || Object.hasOwn(original, 'models')) prices.models = Object.fromEntries(models);
      return Object.keys(prices).length ? prices : undefined;
    },
  };
}

// A row represents one public name, not one upstream ID. Never key rows by ID:
// [{upstream: 'alias-a'}, {upstream: 'alias-b'}] must retain both mappings.
export function modelsToRows(models = []) {
  if (!Array.isArray(models)) throw new Error('已有模型配置不是数组，无法安全编辑。');
  return models.flatMap(model => {
    if (typeof model === 'string') return [{ upstream: model, alias: '' }];
    if (!model || Array.isArray(model) || typeof model !== 'object' || !Object.keys(model).length) throw new Error('已有模型配置包含无法识别的条目，请先检查源配置。');
    return Object.entries(model).map(([upstream, alias]) => {
      if (typeof alias !== 'string') throw new Error(`模型 ${upstream} 的别名不是字符串，无法安全编辑。`);
      return { upstream, alias };
    });
  });
}
export function rowsToModels(rows) {
  return rows.map((row, index) => {
    const upstream = row.upstream.trim(), alias = row.alias.trim();
    if (!upstream || upstream === 'all') throw new Error(`第 ${index + 1} 个模型需要明确的上游 ID，不能使用 all。`);
    if (/\s/.test(upstream) || /\s/.test(alias)) throw new Error(`模型 ${upstream} 的 ID 和对外别名不能包含空白字符。`);
    return alias ? { [upstream]: alias } : upstream;
  });
}
export function splitModelInput(value) {
  return [...new Set(value.split(/[\s,，;；]+/u).map(id => id.trim()).filter(Boolean))];
}
export function mergeModelCandidates(current, incoming) {
  const result = new Map(current.map(model => [model.id, { ...model }]));
  for (const model of incoming) {
    const id = typeof model === 'string' ? model : model?.id;
    if (typeof id !== 'string' || !id.trim()) continue;
    const next = typeof model === 'string' ? { id } : { id, ...(typeof model.name === 'string' ? { name: model.name } : {}) };
    result.set(id, { ...result.get(id), ...next });
  }
  return [...result.values()];
}
export function selectModelCandidates(rows, ids, selected) {
  const targets = new Set(ids);
  if (!selected) return rows.filter(row => !targets.has(row.upstream));
  const existing = new Set(rows.map(row => row.upstream));
  return [...rows, ...[...targets].filter(id => !existing.has(id)).map(upstream => ({ upstream, alias: '' }))];
}
export function matchingModelRows(rows, query) {
  const term = query.trim().toLowerCase();
  return rows.map((row, index) => ({ row, index })).filter(({ row }) => !term || `${row.upstream} ${row.alias}`.toLowerCase().includes(term));
}

const PRESET_GROUPS = {
  openai: ['gpt-5.4', 'gpt-5.4-mini', 'gpt-5.3-codex', 'gpt-5.2', 'gpt-4.1', 'gpt-4o', 'o3', 'o4-mini'],
  claude: ['claude-opus-4-6', 'claude-sonnet-4-6', 'claude-haiku-4-5-20251001', 'claude-sonnet-4-5-20250929', 'claude-3-7-sonnet-20250219'],
  gemini: ['gemini-3.1-pro-preview', 'gemini-3-flash-preview', 'gemini-2.5-pro', 'gemini-2.5-flash', 'gemini-2.0-flash'],
  antigravity: ['claude-opus-4-6-thinking', 'claude-sonnet-4-6', 'claude-sonnet-4-5', 'gemini-3.1-pro-high', 'gemini-3-flash', 'gemini-2.5-pro'],
  cn: ['deepseek-chat', 'deepseek-reasoner', 'qwen-max-latest', 'glm-4.5', 'kimi-k2-0711-preview'],
};

export function createModelsEditor(root, { models = [], pricing = {}, discover, testModel, onChange = () => {}, onPricingChange = onChange }) {
  let rows = modelsToRows(models);
  const modelPricing = createModelPricing(pricing);
  const testResults = new WeakMap();
  let candidates = mergeModelCandidates([], rows.map(row => row.upstream));
  let sequence = 0;
  let requestController;
  let destroyed = false;
  let catalogVisible = false;
  let displayedRows = [];
  root.innerHTML = `<div class="models-heading"><div><h3 id="models-title">模型与映射</h3></div><div class="table-actions"><button type="button" class="button primary" data-model-action="discover">${icon('refresh')}获取模型</button></div></div>
    <p class="models-message" role="status" aria-live="polite" hidden></p>
    <div class="models-catalog" hidden><div class="models-search search-field">${icon('search')}<input type="search" aria-label="搜索候选模型" placeholder="搜索模型名称或 ID" data-model-search></div><div class="models-catalog-actions"><span data-candidate-count></span><div><button class="button small ghost" type="button" data-model-action="select-filtered">全选</button><button class="button small ghost" type="button" data-model-action="clear-filtered">取消</button></div></div><div class="model-candidates" role="group" aria-label="候选模型"></div></div>
    <div class="models-manual"><div class="models-manual-head"><label for="manual-model-input">手动添加（支持逗号/换行批量粘贴，或 <code class="mono">上游模型=对外名称</code>）</label><div class="models-presets"><span>常用预设：</span><button type="button" class="button small ghost" data-model-action="preset" data-preset="openai">+ GPT/Codex</button><button type="button" class="button small ghost" data-model-action="preset" data-preset="claude">+ Claude</button><button type="button" class="button small ghost" data-model-action="preset" data-preset="gemini">+ Gemini</button><button type="button" class="button small ghost" data-model-action="preset" data-preset="antigravity">+ 反重力</button><button type="button" class="button small ghost" data-model-action="preset" data-preset="cn">+ DeepSeek/国产</button></div></div><div><input id="manual-model-input" autocomplete="off" spellcheck="false" placeholder="模型 ID（多个用逗号分隔）或 上游ID=对外别名"><button type="button" class="button" data-model-action="add">${icon('plus')}添加</button><button type="button" class="button ghost" data-model-action="strip-prefix">去除厂商前缀</button></div></div>
    <div class="models-selected-heading"><h3>已选模型 <span class="badge purple" data-selected-count aria-live="polite"></span></h3><div class="table-actions">${testModel ? `<button type="button" class="button small primary" data-model-action="test-all">${icon('pulse')}批量测活</button><button type="button" class="button small text-danger" data-model-action="remove-failed" hidden>${icon('trash')}移除失败</button>` : ''}<button type="button" class="button small ghost" data-model-action="copy-all">${icon('copy')}复制全部</button><button type="button" class="button small ghost text-danger" data-model-action="clear-all">${icon('trash')}清空</button></div></div>
    <div class="models-selected-search"><div class="search-field">${icon('search')}<input type="search" data-selected-search placeholder="搜索已选模型或别名" aria-label="搜索已选模型或别名"></div><button type="button" class="button small" data-model-action="clear-selected-filter" hidden>清除筛选</button></div>
    <div class="models-tools-grid">${testModel ? `<details class="test-params-details"><summary>自定义测活参数（默认发 "hi"）</summary><div class="models-test-bar"><label class="test-param-item"><span>测试内容</span><input type="text" data-model-test-prompt value="hi" placeholder="hi"></label><label class="test-param-item"><span>温度 (Temp)</span><input type="number" data-model-test-temp step="0.1" min="0" max="2" placeholder="默认不传"></label><label class="test-param-item"><span>Top P</span><input type="number" data-model-test-top-p step="0.05" min="0" max="1" placeholder="默认不传"></label><label class="test-param-item"><span>最大 Token</span><input type="number" data-model-test-limit value="16" min="1" step="1" placeholder="16"></label><label class="test-param-item"><span>思考强度</span><select data-model-test-effort><option value="">默认</option><option value="low">low</option><option value="medium">medium</option><option value="high">high</option></select></label></div></details>` : ''}<details class="model-batch-pricing"><summary>批量设置价格</summary><div class="model-price-fields">${PRICE_FIELDS.map(([key, label]) => `<label><span>${label}</span><input type="text" inputmode="decimal" autocomplete="off" data-batch-price-key="${key}" placeholder="保持原值" aria-label="批量设置${label}单价"></label>`).join('')}</div><div class="model-batch-actions"><span class="hint">只更新填写项；0 为免费。保存渠道后生效。</span><button type="button" class="button small" data-model-action="apply-prices"></button></div><p class="models-message" data-batch-price-message role="status" hidden></p></details></div>
    ${PRICE_FIELDS.some(([key]) => pricing[key] != null) ? '<p class="hint">已有渠道默认价格已载入各模型，修改仅对当前模型生效。</p>' : ''}<div class="model-selected-list"></div>`;
  const search = root.querySelector('[data-model-search]');
  const selectedSearch = root.querySelector('[data-selected-search]');
  const manual = root.querySelector('#manual-model-input');
  const message = root.querySelector('.models-message');
  const discoverButton = root.querySelector('[data-model-action="discover"]');
  const visibleCandidates = () => {
    const query = search.value.trim().toLowerCase();
    return candidates.filter(model => `${model.id} ${model.name || ''}`.toLowerCase().includes(query));
  };
  function showMessage(text, error = false) {
    message.textContent = text;
    message.hidden = !text;
    message.classList.toggle('is-error', error);
    message.setAttribute('role', error ? 'alert' : 'status');
  }
  function renderCandidates() {
    root.querySelector('.models-catalog').hidden = !catalogVisible;
    const visible = visibleCandidates();
    const selected = new Set(rows.map(row => row.upstream));
    root.querySelector('[data-candidate-count]').textContent = `${visible.length} / ${candidates.length} 个候选`;
    root.querySelector('[data-model-action="select-filtered"]').disabled = !visible.length;
    root.querySelector('[data-model-action="clear-filtered"]').disabled = !visible.some(model => selected.has(model.id));
    root.querySelector('.model-candidates').innerHTML = visible.length ? visible.map((model, index) => `<label class="model-candidate"><input type="checkbox" data-candidate-index="${index}" ${selected.has(model.id) ? 'checked' : ''}>${aiIcon(modelBrand(model.id))}<span><strong>${esc(model.id)}</strong>${model.name && model.name !== model.id ? `<small>${esc(model.name)}</small>` : ''}</span></label>`).join('') : `<p class="models-empty">${candidates.length ? '没有匹配的候选模型。' : '点击“获取模型”读取目录，或在下方手动添加。'}</p>`;
  }
  function formatRowTest(state) {
    if (!state) return '';
    if (state.status === 'testing') return `<div class="model-test-status">${badge('正在发 hi…', 'purple')}</div>`;
    if (state.status === 'ok') return `<div class="model-test-status">${badge(`${fmt(state.elapsed_ms)} ms`, 'green')}${state.reply ? `<span class="hint">${esc(state.reply.slice(0, 90))}</span>` : ''}</div>`;
    return `<div class="model-test-status">${badge(`HTTP ${state.http_status || 'ERR'}`, 'orange')}<span class="text-danger">${esc((state.error || '失败').slice(0, 120))}</span></div>`;
  }
  function getTestOptions() {
    return {
      prompt: root.querySelector('[data-model-test-prompt]')?.value || 'hi',
      temperature: root.querySelector('[data-model-test-temp]')?.value ?? '',
      topP: root.querySelector('[data-model-test-top-p]')?.value ?? '',
      limit: root.querySelector('[data-model-test-limit]')?.value ?? '16',
      reasoningEffort: root.querySelector('[data-model-test-effort]')?.value ?? '',
    };
  }
  async function runRowTest(row) {
    if (!testModel) return;
    const modelName = row.alias.trim() || row.upstream.trim();
    if (!modelName) { showMessage('请先填写模型 ID 后再测活。', true); return; }
    testResults.set(row, { status: 'testing' });
    renderSelected();
    try {
      const res = await testModel(modelName, getTestOptions());
      if (destroyed) return;
      const reply = res.response?.choices?.map(c => c.message?.content || c.message?.reasoning_content || '').join(' ').trim()
        || (res.response?.output || []).flatMap(i => i.content || []).map(p => p.text || '').join(' ').trim()
        || '';
      if (res.ok) {
        testResults.set(row, { status: 'ok', http_status: res.http_status, elapsed_ms: res.elapsed_ms, reply });
      } else {
        const errText = res.response?.error?.message || res.response?.message || res.response?.detail || res.response?.text || '请求失败';
        testResults.set(row, { status: 'error', http_status: res.http_status, elapsed_ms: res.elapsed_ms, error: errText });
      }
    } catch (err) {
      if (destroyed) return;
      testResults.set(row, { status: 'error', http_status: 'ERR', error: err.message });
    }
    renderSelected();
  }
  async function runBatchRowTests() {
    const queue = displayedRows.map(({ row }) => row);
    if (!queue.length) return;
    let cursor = 0;
    const workers = Array.from({ length: Math.min(3, queue.length) }, async () => {
      while (!destroyed) {
        const idx = cursor++;
        if (idx >= queue.length) break;
        await runRowTest(queue[idx]);
      }
    });
    await Promise.all(workers);
  }
  function renderSelected() {
    displayedRows = matchingModelRows(rows, selectedSearch.value);
    root.querySelector('[data-selected-count]').textContent = displayedRows.length === rows.length ? rows.length : `${displayedRows.length} / ${rows.length}`;
    root.querySelector('[data-model-action="clear-selected-filter"]').hidden = !selectedSearch.value;
    const removeFailedBtn = root.querySelector('[data-model-action="remove-failed"]');
    if (removeFailedBtn) {
      const failedCount = rows.filter(r => testResults.get(r)?.status === 'error').length;
      removeFailedBtn.hidden = failedCount === 0;
      removeFailedBtn.innerHTML = `${icon('trash')}移除失败 (${failedCount})`;
    }
    const testAllBtn = root.querySelector('[data-model-action="test-all"]');
    if (testAllBtn) {
      testAllBtn.disabled = !displayedRows.length;
      testAllBtn.innerHTML = `${icon('pulse')}批量测活 (${displayedRows.length})`;
    }
    const applyButton = root.querySelector('[data-model-action="apply-prices"]');
    applyButton.disabled = !displayedRows.length;
    applyButton.textContent = `应用到当前 ${displayedRows.length} 项`;
    root.querySelector('.model-selected-list').innerHTML = displayedRows.length ? displayedRows.map(({ row, index }) => `<div class="model-selected-row"><label class="model-selected-id"><span class="model-id-caption">${aiIcon(modelBrand(row.upstream))}上游 ID</span><input data-upstream-index="${index}" value="${esc(row.upstream)}" placeholder="上游模型 ID" autocomplete="off" spellcheck="false" aria-label="第 ${index + 1} 项的上游模型 ID"></label><label class="model-alias-field"><span>对外别名</span><input data-alias-index="${index}" value="${esc(row.alias)}" placeholder="与上游 ID 相同" autocomplete="off" spellcheck="false" aria-label="${esc(row.upstream)} 的对外别名，第 ${index + 1} 项"></label><div class="model-row-actions">${testModel ? `<button class="button small" type="button" data-model-action="test-row" data-row-index="${index}" title="发送测试验证模型连通性">${icon('pulse')}测活</button>` : ''}<button class="icon-button" type="button" data-model-action="alias" data-row-index="${index}" title="为此模型添加另一个别名" aria-label="为 ${esc(row.upstream)} 添加另一个别名">${icon('plus')}</button><button class="icon-button" type="button" data-model-action="remove" data-row-index="${index}" title="移除此项" aria-label="移除 ${esc(row.alias || row.upstream)}">${icon('close')}</button></div>${formatRowTest(testResults.get(row))}${priceFields(row, index)}</div>`).join('') : `<p class="models-empty">${rows.length ? '没有匹配的已选模型。' : '尚未选择模型。获取目录后勾选，或直接输入模型 ID。'}</p>`;
  }
  function applyBatchPrices() {
    const status = root.querySelector('[data-batch-price-message]');
    const values = Object.fromEntries([...root.querySelectorAll('[data-batch-price-key]')].map(input => [input.dataset.batchPriceKey, input.value]));
    try {
      const targets = displayedRows.map(({ row }) => row);
      modelPricing.apply(targets, values);
      status.textContent = `已更新 ${targets.length} 项，保存渠道后生效。`;
      status.classList.remove('is-error'); status.setAttribute('role', 'status');
      renderSelected(); onPricingChange();
    } catch (error) { status.textContent = error.message; status.classList.add('is-error'); status.setAttribute('role', 'alert'); }
    status.hidden = false;
  }
  function priceFields(row, index) {
    const state = modelPricing.entry(row);
    return `<details class="model-pricing" data-price-row="${index}"${state.expanded ? ' open' : ''}><summary>价格 <span>USD / 百万 Token</span></summary><div class="model-price-fields">${PRICE_FIELDS.map(([key, label]) => `<label><span>${label}</span><input type="number" min="0" max="1000000" step="any" data-price-index="${index}" data-price-key="${key}" value="${esc(state.values[key])}" placeholder="未配置" aria-label="第 ${index + 1} 个模型的${label}单价"></label>`).join('')}</div><p class="hint">0 为免费；留空不计价。改价不影响历史费用。</p></details>`;
  }
  function render() { renderCandidates(); renderSelected(); onChange(); }
  function choose(ids, selected) {
    rows = selectModelCandidates(rows, ids, selected);
    render();
  }
  function addManual(value) {
    const tokens = splitModelInput(value);
    if (!tokens.length) { manual.focus(); return; }
    const parsedPairs = tokens.map(token => {
      const eq = token.indexOf('=');
      if (eq > 0) return { upstream: token.slice(0, eq).trim(), alias: token.slice(eq + 1).trim() };
      return { upstream: token, alias: '' };
    }).filter(item => item.upstream);
    const ids = parsedPairs.map(item => item.upstream);
    if (ids.includes('all')) { showMessage('请填写具体模型 ID，不能使用 all。', true); return; }
    const before = rows.length;
    for (const pair of parsedPairs) {
      if (pair.alias) {
        if (!rows.some(r => r.upstream === pair.upstream && r.alias === pair.alias)) rows.push(pair);
      } else if (!rows.some(r => r.upstream === pair.upstream)) {
        rows.push(pair);
      }
    }
    candidates = mergeModelCandidates(candidates, ids);
    manual.value = '';
    selectedSearch.value = '';
    showMessage(rows.length > before ? `已添加 ${rows.length - before} 个模型映射。` : '这些模型已选中；原有别名保持不变。');
    render(); manual.focus();
  }
  async function fetchModels() {
    const current = ++sequence;
    requestController?.abort(); requestController = new AbortController();
    discoverButton.disabled = true; discoverButton.innerHTML = `${icon('refresh')}正在获取…`;
    root.querySelector('.models-catalog').setAttribute('aria-busy', 'true');
    showMessage('正在读取模型目录，已有选择与别名保持不变。');
    try {
      const result = await discover(requestController.signal);
      if (destroyed || current !== sequence) return;
      candidates = mergeModelCandidates(candidates, result.models);
      catalogVisible = true;
      const sources = { upstream: '上游服务', plugin: '渠道插件', catalog: '内置目录' };
      const discoveredIds = result.models.map(m => typeof m === 'string' ? m : m?.id).filter(Boolean);
      if (result.fallback_reason) {
        showMessage(`上游未开放模型列表接口，已为您载入 ${result.models.length} 个内置常用候选模型，请勾选需要的模型。`);
        renderCandidates();
      } else if (!rows.length && discoveredIds.length) {
        rows = selectModelCandidates(rows, discoveredIds, true);
        showMessage(`已从${sources[result.source] || '服务端'}获取并自动全选 ${discoveredIds.length} 个模型。`);
        render();
      } else {
        showMessage(`已从${sources[result.source] || '服务端'}获取 ${result.models.length} 个模型。请勾选需要的模型，已有选择未更改。`);
        renderCandidates();
      }
    } catch (error) {
      if (!destroyed && current === sequence) showMessage(`${error.message} 可继续手动添加模型。`, true);
    } finally {
      if (!destroyed && current === sequence) { discoverButton.disabled = false; discoverButton.innerHTML = `${icon('refresh')}获取模型`; root.querySelector('.models-catalog').removeAttribute('aria-busy'); }
    }
  }
  root.addEventListener('input', event => {
    if (event.target === search) renderCandidates();
    if (event.target === selectedSearch) renderSelected();
    if (event.target.matches('[data-alias-index]')) { rows[Number(event.target.dataset.aliasIndex)].alias = event.target.value; onChange(); }
    if (event.target.matches('[data-upstream-index]')) { rows[Number(event.target.dataset.upstreamIndex)].upstream = event.target.value; onChange(); }
    if (event.target.matches('[data-price-index]')) { modelPricing.entry(rows[Number(event.target.dataset.priceIndex)]).values[event.target.dataset.priceKey] = event.target.value; onPricingChange(); }
  });
  root.addEventListener('keydown', event => {
    if (event.key !== 'Enter' || event.isComposing || event.ctrlKey || event.metaKey) return;
    if (event.target === search || event.target === selectedSearch) event.preventDefault();
    else if (event.target.matches('[data-batch-price-key]')) { event.preventDefault(); if (displayedRows.length) applyBatchPrices(); }
  });
  root.addEventListener('toggle', event => {
    if (event.target.matches('[data-price-row]')) modelPricing.entry(rows[Number(event.target.dataset.priceRow)]).expanded = event.target.open;
  }, true);
  root.addEventListener('change', event => {
    if (event.target.matches('[data-upstream-index]')) {
      const row = rows[Number(event.target.dataset.upstreamIndex)];
      row.upstream = event.target.value.trim(); event.target.value = row.upstream;
      event.target.closest('.model-selected-id').querySelector('.model-id-caption').innerHTML = `${aiIcon(modelBrand(row.upstream))}上游 ID`;
      if (row.upstream) candidates = mergeModelCandidates(candidates, [row.upstream]);
      renderCandidates(); // Do not rebuild the selected row or disturb its edit focus.
      return;
    }
    if (!event.target.matches('[data-candidate-index]')) return;
    const candidate = visibleCandidates()[Number(event.target.dataset.candidateIndex)];
    if (candidate) choose([candidate.id], event.target.checked);
  });
  root.addEventListener('click', event => {
    const button = event.target.closest('[data-model-action]');
    if (!button || button.disabled) return;
    const action = button.dataset.modelAction;
    if (action === 'discover') fetchModels();
    else if (action === 'add') addManual(manual.value);
    else if (action === 'preset') {
      const preset = PRESET_GROUPS[button.dataset.preset] || [];
      if (preset.length) addManual(preset.join(', '));
    }
    else if (action === 'strip-prefix') {
      let updated = 0;
      for (const row of rows) {
        if (!row.alias && row.upstream.includes('/')) {
          const shortName = row.upstream.slice(row.upstream.lastIndexOf('/') + 1).trim();
          if (shortName) { row.alias = shortName; updated++; }
        }
      }
      showMessage(updated ? `已为 ${updated} 个带前缀的模型自动生成简短对外名称。` : '当前没有需要去除 vendor/ 前缀的模型。');
      render();
    }
    else if (action === 'clear-selected-filter') { selectedSearch.value = ''; renderSelected(); selectedSearch.focus(); }
    else if (action === 'apply-prices') applyBatchPrices();
    else if (action === 'select-filtered' || action === 'clear-filtered') choose(visibleCandidates().map(model => model.id), action === 'select-filtered');
    else if (action === 'test-row') { const row = rows[Number(button.dataset.rowIndex)]; if (row) runRowTest(row); }
    else if (action === 'test-all') runBatchRowTests();
    else if (action === 'remove-failed') {
      const before = rows.length;
      rows = rows.filter(r => testResults.get(r)?.status !== 'error');
      showMessage(`已移除 ${before - rows.length} 个测活失败的模型。`);
      render();
    }
    else if (action === 'copy-all') {
      const names = rows.map(r => r.alias.trim() || r.upstream.trim()).filter(Boolean);
      if (!names.length) { showMessage('当前没有可复制的模型。', true); return; }
      navigator.clipboard?.writeText(names.join(', ')).then(() => showMessage(`已复制 ${names.length} 个模型名称到剪贴板。`), () => showMessage(names.join(', ')));
    }
    else if (action === 'clear-all') {
      if (!rows.length) return;
      rows = [];
      showMessage('已清空全部已选模型。');
      render();
    }
    else if (action === 'remove') { rows.splice(Number(button.dataset.rowIndex), 1); render(); }
    else if (action === 'alias') { const index = Number(button.dataset.rowIndex), copy = { upstream: rows[index].upstream, alias: '' }; modelPricing.copy(rows[index], copy); rows.splice(index + 1, 0, copy); selectedSearch.value = ''; renderSelected(); onChange(); root.querySelector(`[data-alias-index="${index + 1}"]`).focus(); }
  });
  manual.addEventListener('keydown', event => { if (event.key === 'Enter' && !event.isComposing) { event.preventDefault(); addManual(manual.value); } });
  manual.addEventListener('paste', event => { const text = event.clipboardData?.getData('text') || ''; if (splitModelInput(text).length > 1) { event.preventDefault(); addManual([manual.value, text].join('\n')); } });
  render();
  return {
    discover: fetchModels,
    snapshot() { return rows.map(row => ({ ...row })); },
    pricing() { return modelPricing.value(rows); },
    value() {
      const model = rowsToModels(rows);
      if (!model.length) throw new Error('请至少选择或手动添加一个模型。');
      const publicNames = new Set();
      for (const row of rows) { const name = row.alias.trim() || row.upstream.trim(); if (publicNames.has(name)) throw new Error(`对外模型名“${name}”重复，请为各项设置不同的别名。`); publicNames.add(name); }
      return model;
    },
    resetCandidates() {
      sequence++; requestController?.abort();
      catalogVisible = false;
      candidates = mergeModelCandidates([], rows.map(row => row.upstream)); search.value = '';
      discoverButton.disabled = false; discoverButton.innerHTML = `${icon('refresh')}获取模型`; root.querySelector('.models-catalog').removeAttribute('aria-busy');
      showMessage('渠道类型已更改，已选模型和别名均已保留。请确认模型兼容性，或重新获取目录。'); renderCandidates();
    },
    destroy() { destroyed = true; sequence++; requestController?.abort(); rows = []; candidates = []; },
  };
}
