import { icon } from './icons.js';
import { esc, field, select, parseJSON } from './ui.js';

export const clonePolicy = value => value === undefined ? {} : JSON.parse(JSON.stringify(value));
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const own = (value, key) => object(value) && Object.hasOwn(value, key);
function pointerParts(path) {
  if (typeof path !== 'string' || !path.startsWith('/') || path === '/') throw new Error('参数能力返回了无效的 JSON Pointer。');
  const parts = path.slice(1).split('/').map(part => part.replace(/~1/g, '/').replace(/~0/g, '~'));
  if (parts.some(part => ['__proto__', 'prototype', 'constructor'].includes(part))) throw new Error('不允许修改此参数路径。');
  return parts;
}
export function readPolicyPath(value, path) {
  let current = value;
  for (const part of pointerParts(path)) { if (!own(current, part)) return { exists: false }; current = current[part]; }
  return { exists: true, value: current };
}
function writePath(root, parts, value, remove) {
  const key = parts[0];
  if (parts.length === 1) { if (remove) delete root[key]; else root[key] = value; return; }
  if (!object(root[key])) { if (remove) return; root[key] = {}; }
  writePath(root[key], parts.slice(1), value, remove);
  if (remove && !Object.keys(root[key]).length) delete root[key];
}
export function parameterState(policy, path) {
  const removed = Array.isArray(policy?.channel?.remove) && policy.channel.remove.includes(path);
  const direct = readPolicyPath(policy?.channel?.set, path);
  const fallback = readPolicyPath(policy?.defaults, path);
  return { mode: removed ? 'remove' : direct.exists ? 'set' : 'inherit', ...(direct.exists ? { value: direct.value } : {}), ...(fallback.exists ? { inherited: fallback.value } : {}) };
}
export function patchParameter(policy, path, mode, value) {
  if (!['inherit', 'set', 'remove'].includes(mode)) throw new Error('未知参数模式。');
  const next = clonePolicy(policy);
  if (!object(next.channel)) next.channel = {};
  if (!object(next.channel.set)) next.channel.set = {};
  writePath(next.channel.set, pointerParts(path), clonePolicyValue(value), mode !== 'set');
  const remove = Array.isArray(next.channel.remove) ? next.channel.remove.filter(item => item !== path) : [];
  if (mode === 'remove') remove.push(path);
  next.channel.remove = remove;
  return next;
}
function clonePolicyValue(value) { return value === undefined ? undefined : JSON.parse(JSON.stringify(value)); }
const budgetPaths = ['/max_tokens', '/max_completion_tokens', '/max_output_tokens', '/generationConfig/maxOutputTokens'];
export function generationState(policy, name, spec) {
  if (name !== 'output_limit') return parameterState(policy, spec.path);
  const ordered = [spec.path, ...budgetPaths.filter(path => path !== spec.path)];
  const removed = ordered.some(path => Array.isArray(policy?.channel?.remove) && policy.channel.remove.includes(path));
  const direct = ordered.map(path => readPolicyPath(policy?.channel?.set, path)).find(value => value.exists);
  const fallback = ordered.map(path => readPolicyPath(policy?.defaults, path)).find(value => value.exists);
  return { mode: removed ? 'remove' : direct ? 'set' : 'inherit', ...(direct ? { value: direct.value } : {}), ...(fallback ? { inherited: fallback.value } : {}) };
}
export function patchGeneration(policy, name, spec, mode, value) {
  let next = clonePolicy(policy);
  if (name === 'output_limit') {
    for (const path of budgetPaths) {
      if (path !== spec.path && readPolicyPath(next.channel?.set, path).exists) {
        writePath(next.channel.set, pointerParts(path), undefined, true);
      }
    }
    if (Array.isArray(next.channel?.remove)) next.channel.remove = next.channel.remove.filter(path => !budgetPaths.includes(path));
  }
  return patchParameter(next, spec.path, mode, value);
}
export function parseParameterValue(text) {
  if (text === '') return '';
  try { return JSON.parse(text); } catch { return text; }
}
export function patchSystemPrompt(policy, mode, value) {
  const next = clonePolicy(policy);
  next.system_prompt = { ...(object(next.system_prompt) ? next.system_prompt : {}), mode };
  if (['prepend', 'append', 'replace'].includes(mode)) next.system_prompt.value = value;
  return next;
}
export function sensitiveHeader(name, secret = false) { return secret === true || /authorization|cookie|api[-_]?key|auth[-_]?token|access[-_]?token/i.test(String(name)); }
export function patchHeaderValue(original, changes, { edited = false, value = '' } = {}) {
  const row = { ...clonePolicy(original), ...clonePolicy(changes) };
  if (edited) row.value = value;
  else if (sensitiveHeader(row.name, row.secret)) delete row.value;
  return row;
}
export function policyForAdvanced(policy) {
  const result = clonePolicy(policy);
  if (Array.isArray(result.headers)) result.headers = result.headers.map(header => {
    const row = { ...header }; if (sensitiveHeader(row.name, row.secret)) delete row.value; return row;
  });
  return result;
}
// Advanced JSON is the complete policy. Only omitted/placeholder sensitive
// header values are restored from this session's draft; no known section is rebuilt.
export function assertUniqueHeaders(headers) {
  if (!Array.isArray(headers)) return;
  const names = new Set();
  headers.forEach((row, index) => {
    if (!object(row) || typeof row.name !== 'string' || !row.name.trim()) throw new Error(`请填写第 ${index + 1} 条请求头的名称。`);
    const name = row.name.trim().toLowerCase();
    if (names.has(name)) throw new Error(`请求头“${row.name}”重复（名称不区分大小写）。请保留一条规则，避免敏感值错配。`);
    names.add(name);
  });
}
export function applyAdvancedPolicy(text, current) {
  const next = parseJSON(text, '完整参数策略');
  assertUniqueHeaders(next.headers);
  if (Array.isArray(next.headers)) next.headers = next.headers.map(row => {
    if (!object(row)) throw new Error('请求头条目必须是对象。');
    const matches = (Array.isArray(current.headers) ? current.headers : []).filter(old => String(old.name).toLowerCase() === String(row.name).toLowerCase());
    if (sensitiveHeader(row.name, row.secret) && (!own(row, 'value') || row.value === '***' || row.value === '********')) {
      if (matches.length > 1) throw new Error(`原策略有多个“${row.name}”，无法安全保留敏感值。请明确填写保留行的新值。`);
      if (own(matches[0], 'value')) return { ...row, value: matches[0].value };
    }
    return row;
  });
  return clonePolicy(next);
}
const labels = { temperature: '温度 Temperature', top_p: '采样 Top P', output_limit: '最大输出 Token', stop: '停止序列 Stop' };
const modeOptions = [{ value: 'inherit', label: '继承 · 不写渠道覆盖' }, { value: 'set', label: '设置 · 使用指定值' }, { value: 'remove', label: '删除 · 移除请求字段' }];
const promptLabels = { inherit: '继承 · 保持原系统消息', prepend: '前置 · 加在现有提示之前', append: '追加 · 加在现有提示之后', replace: '替换 · 替换现有系统提示', remove: '移除 · 删除系统提示' };
const valueText = value => value === undefined ? '' : JSON.stringify(value);

export function createParametersEditor(root, { policy, onChange = () => {} } = {}) {
  let draft = clonePolicy(policy), dirty = false, capabilities = null;
  let headerRows = [];
  let advancedDirty = false;
  let modelRulesDirty = false;
  let destroyed = false;
  root.innerHTML = `<div class="parameters-intro"><h3>参数与请求修饰</h3><p>仅展示当前供应商和请求端点真实支持的字段。继承、设置与删除互不混用。</p></div><div class="parameter-capability-state" role="status">正在读取参数能力…</div><div class="parameter-generation"></div><section class="parameter-section"><div class="parameter-section-heading"><h3>请求头</h3><button type="button" class="button small" data-parameter-action="add-header">${icon('plus')}添加请求头</button></div><p class="hint parameter-header-note"></p><div class="parameter-headers"></div></section><section class="parameter-section parameter-system"></section><details class="advanced parameter-model-rules"><summary>按公开模型覆盖</summary><p class="hint">精确匹配 requested_model（对外别名），按数组顺序应用。未匹配的模型不受影响。</p>${field('policy_models', '模型级覆盖规则 JSON', JSON.stringify(draft.models ?? [], null, 2), { type: 'textarea', help: '条目包含 match.requested_model、set 嵌套字段及 remove JSON Pointer 数组。其他扩展字段会保留。' })}<button type="button" class="button small" data-parameter-action="apply-models">${icon('check')}应用模型规则</button></details><details class="advanced parameter-advanced"><summary>完整策略 JSON</summary><p class="hint">普通表单只修改已知字段，保留未知扩展。此处是完整策略；应用时已知段按所填内容替换。敏感请求头的值省略表示保留，显式 value: "" 表示清空。</p>${field('policy_json', 'preferences.parameter_policy', JSON.stringify(policyForAdvanced(draft), null, 2), { type: 'textarea', rows: 10 })}<button type="button" class="button small" data-parameter-action="apply-json">${icon('check')}应用完整 JSON</button></details><p class="parameter-editor-message hint" role="status" hidden></p>`;
  const advancedInput = root.querySelector('[name="policy_json"]');
  const rulesInput = root.querySelector('[name="policy_models"]');
  const message = root.querySelector('.parameter-editor-message');
  function inform(text, error = false) { message.textContent = text; message.hidden = !text; message.classList.toggle('text-danger', error); message.setAttribute('role', error ? 'alert' : 'status'); }
  function markChanged() { dirty = true; onChange(); }
  function loadHeaders() {
    headerRows = Array.isArray(draft.headers) ? draft.headers.map(raw => ({ raw: clonePolicy(raw), edited: false, input: sensitiveHeader(raw.name, raw.secret) ? '' : String(raw.value ?? ''), saved: own(raw, 'value') })) : [];
  }
  function collected() {
    const next = clonePolicy(draft);
    if (Array.isArray(draft.headers)) next.headers = headerRows.map(row => patchHeaderValue(row.raw, {}, { edited: row.edited, value: row.input }));
    return next;
  }
  function syncAdvanced() { if (!advancedDirty) advancedInput.value = JSON.stringify(policyForAdvanced(collected()), null, 2); }
  function renderGeneration() {
    const fields = Object.entries(capabilities?.generation || {}).filter(([name, spec]) => Object.hasOwn(labels, name) && spec.supported === true && typeof spec.path === 'string');
    root.querySelector('.parameter-generation').innerHTML = `<section class="parameter-section"><div class="parameter-section-heading"><h3>生成参数</h3><span class="hint">仅覆盖当前渠道</span></div>${fields.length ? `<div class="parameter-fields">${fields.map(([name, spec]) => {
      const state = generationState(draft, name, spec);
      const bounds = [Number.isFinite(spec.minimum) ? `最小 ${spec.minimum}` : '', Number.isFinite(spec.maximum) ? `最大 ${spec.maximum}` : '', spec.nullable ? '允许 null 值' : '不接受 null'].filter(Boolean).join(' · ');
      return `<div class="parameter-field" data-generation="${name}"><div class="parameter-field-title"><label for="parameter-value-${name}">${labels[name]}</label><code>${esc(spec.path)}</code></div><select data-parameter-mode="${name}" aria-label="${labels[name]}覆盖模式">${modeOptions.map(option => `<option value="${option.value}" ${state.mode === option.value ? 'selected' : ''}>${option.label}</option>`).join('')}</select><input id="parameter-value-${name}" data-parameter-value="${name}" value="${esc(valueText(state.value))}" ${state.mode !== 'set' ? 'disabled' : ''} placeholder="${esc(state.inherited !== undefined ? `默认层：${valueText(state.inherited)}` : name === 'stop' ? '["STOP"] 或 "STOP"' : '填写数值')}"><small>${esc(bounds)}</small></div>`;
    }).join('')}</div><p class="hint">0、false、空字符串和 null 都是值，不代表删除。删除请单独选择“删除”；只有后端允许的类型和范围才能生效。</p>` : `<p class="hint">${capabilities ? '此端点没有声明可用的生成参数。现有高级策略仍会保留。' : '读取能力后显示可编辑参数；不会猜测供应商字段。'}</p>`}</section>`;
  }
  function headerValueField(row, index) {
    const sensitive = sensitiveHeader(row.raw.name, row.raw.secret);
    return `<label class="parameter-header-value"><span>值</span><input data-header-field="value" data-header-index="${index}" type="${sensitive ? 'password' : 'text'}" autocomplete="new-password" value="${esc(row.edited || !sensitive ? row.input : '')}" ${(row.raw.mode || 'inherit') !== 'set' ? 'disabled' : ''} placeholder="${sensitive && row.saved && !row.edited ? '已保存，留空保留' : '可显式设置为空字符串'}"><small>${sensitive && row.saved && !row.edited ? '已保存的敏感值不会回显。' : row.edited && row.input === '' ? '将显式设置为空字符串。' : sensitive ? '敏感请求头始终隐藏显示。' : '按当前模式处理请求头。'}</small></label><button class="button small" type="button" data-parameter-action="clear-header" data-header-index="${index}" ${(row.raw.mode || 'inherit') !== 'set' ? 'disabled' : ''}>显式清空值</button>`;
  }
  function renderHeaders() {
    const supported = capabilities?.header_supported === true;
    root.querySelector('[data-parameter-action="add-header"]').disabled = !supported;
    root.querySelector('.parameter-header-note').textContent = supported ? '敏感值不会回显；认证头即使未勾选“敏感”也隐藏。关闭“启用”会保留规则但不执行。' : capabilities?.plugin_envelope ? '服务插件仅允许宿主请求体策略，不能覆盖插件控制面 Authorization 或其他控制请求头。已有规则保持原样。' : '此端点未声明请求头覆写能力，已有规则保持原样。';
    root.querySelector('.parameter-headers').innerHTML = headerRows.length ? headerRows.map((row, index) => `<fieldset class="parameter-header-row" ${supported ? '' : 'disabled'}><legend>请求头 ${index + 1}</legend><div class="parameter-header-main"><label><span>名称</span><input data-header-field="name" data-header-index="${index}" value="${esc(row.raw.name || '')}" placeholder="X-Request-Tag" spellcheck="false"></label><label><span>行为</span><select data-header-field="mode" data-header-index="${index}">${['inherit', 'set', 'remove'].map(mode => `<option value="${mode}" ${(row.raw.mode || 'inherit') === mode ? 'selected' : ''}>${{ inherit: '继承', set: '设置', remove: '删除' }[mode]}</option>`).join('')}</select></label><button class="icon-button" type="button" data-parameter-action="remove-header" data-header-index="${index}" aria-label="移除第 ${index + 1} 条请求头规则">${icon('trash')}</button></div><div class="parameter-header-value-wrap">${headerValueField(row, index)}</div><div class="parameter-header-flags"><label><input type="checkbox" data-header-field="enabled" data-header-index="${index}" ${row.raw.enabled !== false ? 'checked' : ''}>启用</label><label><input type="checkbox" data-header-field="secret" data-header-index="${index}" ${row.raw.secret === true ? 'checked' : ''}>敏感值</label></div></fieldset>`).join('') : '<p class="hint">没有额外请求头规则。</p>';
  }
  function renderSystem() {
    const spec = capabilities?.system_prompt;
    const current = object(draft.system_prompt) ? draft.system_prompt : {};
    const modes = spec?.supported ? (spec.modes || []).filter(mode => Object.hasOwn(promptLabels, mode)) : [];
    root.querySelector('.parameter-system').innerHTML = `<div class="parameter-section-heading"><h3>系统提示</h3></div>${modes.length ? `${select('policy_system_mode', '处理方式', modes.map(mode => ({ value: mode, label: promptLabels[mode] })), current.mode ?? 'inherit')}${field('policy_system_value', '系统提示内容', current.value ?? '', { type: 'textarea', help: '前置 / 追加保留原系统消息；替换 / 移除会改变原提示，请明确选择。' })}` : '<p class="hint">此端点未声明系统提示修改能力，现有策略保持不变。</p>'}`;
    const mode = root.querySelector('[name="policy_system_mode"]');
    const input = root.querySelector('[name="policy_system_value"]');
    if (input) input.disabled = ['inherit', 'remove'].includes(mode.value);
  }
  function writeGeneration(name) {
    const spec = capabilities?.generation?.[name]; if (!spec?.supported) return;
    const mode = root.querySelector(`[data-parameter-mode="${name}"]`).value;
    const input = root.querySelector(`[data-parameter-value="${name}"]`);
    draft = patchGeneration(draft, name, spec, mode, parseParameterValue(input.value));
    input.disabled = mode !== 'set'; markChanged(); syncAdvanced();
  }
  root.addEventListener('input', event => {
    const input = event.target;
    if (input === advancedInput) { advancedDirty = true; return; }
    if (input === rulesInput) { modelRulesDirty = true; return; }
    if (input.dataset.parameterValue) writeGeneration(input.dataset.parameterValue);
    if (input.dataset.headerField && input.dataset.headerIndex !== undefined) {
      const row = headerRows[Number(input.dataset.headerIndex)], key = input.dataset.headerField;
      if (key === 'value') { row.input = input.value; row.edited = true; }
      else if (key === 'name') row.raw.name = input.value;
      markChanged(); syncAdvanced();
    }
    if (input.name === 'policy_system_value') { draft.system_prompt = { ...(draft.system_prompt || {}), value: input.value }; markChanged(); syncAdvanced(); }
  });
  root.addEventListener('change', event => {
    const input = event.target;
    if (input.dataset.parameterMode) writeGeneration(input.dataset.parameterMode);
    if (input.dataset.headerField && input.dataset.headerIndex !== undefined) {
      const index = Number(input.dataset.headerIndex), row = headerRows[index], key = input.dataset.headerField;
      if (key === 'enabled' || key === 'secret') row.raw[key] = input.checked;
      if (key === 'mode') row.raw.mode = input.value;
      if (['name', 'mode', 'secret'].includes(key)) input.closest('.parameter-header-row').querySelector('.parameter-header-value-wrap').innerHTML = headerValueField(row, index);
      markChanged(); syncAdvanced();
    }
    if (input.name === 'policy_system_mode') {
      const inputValue = root.querySelector('[name="policy_system_value"]');
      draft = patchSystemPrompt(draft, input.value, inputValue.value);
      inputValue.disabled = ['inherit', 'remove'].includes(input.value);
      markChanged(); syncAdvanced();
    }
  });
  root.addEventListener('click', event => {
    const button = event.target.closest('[data-parameter-action]'); if (!button || button.disabled) return;
    const action = button.dataset.parameterAction;
    try {
      if (action === 'add-header') { if (!Array.isArray(draft.headers)) draft.headers = []; headerRows.push({ raw: { name: '', mode: 'set', enabled: true, secret: false }, input: '', edited: false, saved: false }); markChanged(); renderHeaders(); }
      if (action === 'remove-header') { headerRows.splice(Number(button.dataset.headerIndex), 1); markChanged(); renderHeaders(); }
      if (action === 'clear-header') { const index = Number(button.dataset.headerIndex), row = headerRows[index]; row.input = ''; row.edited = true; button.closest('.parameter-header-row').querySelector('.parameter-header-value-wrap').innerHTML = headerValueField(row, index); markChanged(); }
      if (action === 'apply-models') { draft.models = parseJSON(rulesInput.value, '模型级覆盖规则', 'array'); modelRulesDirty = false; markChanged(); inform('模型规则已应用到草稿，尚未保存。'); }
      if (action === 'apply-json') {
        const supplied = parseJSON(advancedInput.value, '完整参数策略');
        const previousRows = headerRows;
        draft = applyAdvancedPolicy(advancedInput.value, collected());
        advancedDirty = false; modelRulesDirty = false;
        rulesInput.value = JSON.stringify(draft.models ?? [], null, 2);
        loadHeaders();
        headerRows.forEach((row, index) => {
          const incoming = supplied.headers?.[index];
          const previous = previousRows.find(old => String(old.raw.name).toLowerCase() === String(row.raw.name).toLowerCase());
          if (sensitiveHeader(row.raw.name, row.raw.secret) && own(incoming, 'value') && !['***', '********'].includes(incoming.value)) { row.edited = true; row.input = incoming.value; }
          else if (sensitiveHeader(row.raw.name, row.raw.secret) && previous?.edited) { row.edited = true; row.input = previous.input; }
        });
        renderGeneration(); renderHeaders(); renderSystem(); markChanged(); inform('完整策略已应用到草稿，未知字段保留；尚未保存。');
      }
      syncAdvanced();
    } catch (error) { inform(error.message, true); }
  });
  loadHeaders(); renderGeneration(); renderHeaders(); renderSystem();
  return {
    value() {
      if (advancedDirty) throw new Error('完整策略 JSON 已修改，请先点击“应用完整 JSON”。');
      if (modelRulesDirty) throw new Error('模型级规则已修改，请先点击“应用模型规则”。');
      if (!dirty && policy === undefined) return undefined;
      const value = collected(); assertUniqueHeaders(value.headers);
      return value;
    },
    setCapabilities(next) {
      if (destroyed) return;
      capabilities = next;
      const summary = root.querySelector('.parameter-capability-state');
      summary.textContent = next?.plugin_envelope ? '宿主执行信封：仅处理 request.body，不是插件内部最终请求。' : next ? `当前协议 ${next.profile || '未提供'} · ${next.endpoint || ''}` : '尚未读取参数能力。';
      renderGeneration(); renderHeaders(); renderSystem();
    },
    setCapabilityError(error) { root.querySelector('.parameter-capability-state').textContent = `参数能力读取失败：${error.message}。已有策略不会被清空。`; },
    destroy() { destroyed = true; draft = {}; headerRows = []; },
  };
}
