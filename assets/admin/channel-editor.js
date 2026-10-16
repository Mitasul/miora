import * as api from './api.js';
import { parseGroups } from './key-manager.js';
import { icon, aiIcon, channelBrand } from './icons.js';
import { esc, fmt, notice, field, select, checkbox, parseJSON, schemaFields, readSchema, jsonDetails, toast, dialog } from './ui.js';
import { createModelsEditor, rowsToModels } from './models-editor.js';
import { createParametersEditor } from './parameters-editor.js';

const array = value => Array.isArray(value) ? value : [];
export function canPreview(capabilities, endpoint) {
  return capabilities?.body_supported === true && capabilities.preview_supported !== false && array(capabilities.endpoints).includes(endpoint);
}
export function shouldSendBaseURL(existing, value, savedURL) { return Boolean(value) && (!existing || value !== savedURL); }
const endpointLabel = endpoint => ({ '/v1/chat/completions': 'Chat Completions', '/v1/responses': 'Responses', '/v1/messages': 'Messages' })[endpoint] || endpoint;
const sampleBody = endpoint => endpoint.endsWith('/responses') ? { input: '你好，请简短介绍你自己。' } : { messages: [{ role: 'user', content: '你好，请简短介绍你自己。' }] };

function decodeJwtClaims(token) {
  if (typeof token !== 'string' || !token.includes('.')) return {};
  try {
    const payload = token.split('.')[1].replace(/-/g, '+').replace(/_/g, '/');
    return JSON.parse(atob(payload));
  } catch { return {}; }
}

export function extractAccountFromJson(text, engine = 'codex') {
  const data = JSON.parse(text);
  const entry = Array.isArray(data) ? data[0] : data;
  if (!entry || typeof entry !== 'object') throw new Error('账号文件格式无效');
  const tokens = entry.tokens || entry;
  const oauth = entry.claudeAiOauth || tokens;
  const idClaims = decodeJwtClaims(tokens.id_token);
  const accessClaims = decodeJwtClaims(oauth.access_token || oauth.accessToken);
  const authInfo = idClaims['https://api.openai.com/auth'] || accessClaims['https://api.openai.com/auth'] || {};
  const profileInfo = idClaims['https://api.openai.com/profile'] || accessClaims['https://api.openai.com/profile'] || {};
  const email = entry.email || tokens.email || oauth.email || idClaims.email || profileInfo.email || accessClaims.email || '';
  const plan = entry.plan_type || tokens.plan_type || oauth.subscriptionType || authInfo.chatgpt_plan_type || '';
  const accountId = tokens.account_id || entry.account_id || authInfo.chatgpt_account_id || '';
  if (engine === 'codex' && tokens.refresh_token) {
    return { credential: `${accountId},${tokens.refresh_token}`, email, plan, accountId };
  }
  const token = oauth.accessToken || oauth.access_token || entry.api_key || tokens.refresh_token || '';
  if (!token) throw new Error('未在文件中找到有效的 Token 或凭据');
  return { credential: token, email, plan, accountId };
}

export async function openChannelEditor({ existing = null, initialType = '', revision, writable = true, isCurrent = () => true, onSaved = async () => {} }) {
  const [pluginsResult, detailResult, keysResult] = await Promise.allSettled([api.get('/plugins'), existing ? api.get(`/channels/detail?provider=${encodeURIComponent(existing.provider)}`) : Promise.resolve(null), api.get('/api-keys')]);
  if (!isCurrent()) return;
  if (detailResult.status === 'rejected') throw detailResult.reason;
  const detail = detailResult.value;
  const config = detail?.effective || {};
  const plugins = pluginsResult.status === 'fulfilled' ? array(pluginsResult.value.plugins) : [];
  const keysData = keysResult.status === 'fulfilled' ? keysResult.value : {};
  const initialGroups = config.groups || config.preferences?.groups || ['default'];
  let knownGroups = [...new Set(['default', ...array(keysData.groups), ...array(keysData.api_keys).flatMap(k => array(k.groups)), ...initialGroups])].sort();
  if (!writable || existing?.temporary) {
    dialog({ title: `渠道详情 · ${existing?.provider || ''}`, body: `<p>渠道：${esc(existing?.provider)} · 凭据数：${fmt(existing?.api_key_count)}</p>${jsonDetails(detail, '只读源配置（服务端脱敏）')}`, submitLabel: '关闭', onSubmit: async () => {} }); return;
  }
  const currentRevision = detail?.revision ?? revision;
  const savedPlugin = config.channel_plugin || config.preferences?.channel_plugin || existing?.channel_plugin || '';
  const savedPluginConfig = config.plugin_config || config.preferences?.plugin_config || {};
  const savedEngine = config.engine || existing?.engine || '';
  const savedURL = config.base_url || existing?.base_url || '';
  const types = plugins.filter(plugin => ['builtin', 'service'].includes(plugin.kind)).map(plugin => ({
    value: plugin.kind === 'service' ? `service:${plugin.id}` : `builtin:${plugin.manifest?.engine || plugin.id.replace(/^builtin-/, '')}`,
    label: `${plugin.manifest?.name || plugin.id}${plugin.kind === 'service' && !plugin.enabled ? '（已停用）' : ''}`,
    kind: plugin.kind, plugin,
  }));
  let selected = existing ? savedPlugin ? `service:${savedPlugin}` : `builtin:${savedEngine}` : (initialType && types.some(type => type.value === initialType) ? initialType : types.find(type => type.value === 'builtin:codex')?.value || types.find(type => type.value === 'builtin:gpt')?.value || types[0]?.value || '');
  if (existing && !types.some(type => type.value === selected)) types.unshift({ value: selected, label: `${savedPlugin || savedEngine || '自动识别'}（已配置类型）`, kind: savedPlugin ? 'service' : 'builtin', plugin: null });
  const options = existing ? types.filter(type => savedPlugin ? type.value === selected : type.kind === 'builtin') : types;
  const getType = () => types.find(type => type.value === selected);
  const initialURL = existing ? savedURL : getType()?.plugin?.manifest?.default_base_url || '';
  const initialEndpoints = array(getType()?.plugin?.manifest?.capabilities?.endpoints);
  const initialEndpoint = initialEndpoints.includes('/v1/chat/completions') ? '/v1/chat/completions' : initialEndpoints[0] || '';
  const schemaDrafts = new Map();
  let modelsEditor, parametersEditor, form, capabilities = null;
  let schema = [], urlEdited = Boolean(existing), previousType = selected;
  let schemaController, previewController, schemaSequence = 0, previewSequence = 0;
  let closed = false, sampleEdited = false, previewExists = false;
  let currentAuthMode = 'oauth', oauthSession = null, oauthPollTimer = null;

  function connection({ validate = false, forCapabilities = false } = {}) {
    if (!selected) throw new Error('请先选择渠道类型。');
    const channel = {};
    if (getType()?.kind === 'service') {
      channel.channel_plugin = selected.slice('service:'.length);
      channel.plugin_config = readSchema(form, forCapabilities ? schema.map(spec => ({ ...spec, required: false })) : schema, savedPluginConfig);
    } else {
      const value = form.elements.base_url.value.trim();
      if (validate || value) {
        if (!value) throw new Error('请填写上游服务的 Base URL。');
        let url; try { url = new URL(value); } catch { throw new Error('Base URL 不是有效的地址。'); }
        if (!['http:', 'https:'].includes(url.protocol)) throw new Error('Base URL 必须使用 http 或 https。');
      }
      const engine = selected.slice('builtin:'.length);
      channel.engine = engine || null;
      if (shouldSendBaseURL(existing, value, savedURL)) channel.base_url = value;
      if (form.elements.api.value) channel.api = form.elements.api.value;
    }
    return channel;
  }
  function draftChannel({ validate = false, forCapabilities = false, model } = {}) {
    const extra = parseJSON(form.elements.extra.value || '{}', '附加配置');
    for (const key of ['provider', 'base_url', 'model', 'api', 'engine', 'channel_plugin', 'plugin_config', 'groups']) if (Object.hasOwn(extra, key)) throw new Error(`${key} 请使用对应表单配置。`);
    if (extra.preferences !== undefined && (extra.preferences === null || typeof extra.preferences !== 'object' || Array.isArray(extra.preferences))) throw new Error('preferences 必须是对象。');
    if (extra.preferences && ['parameter_policy', 'protocol_mode', 'usage_pricing'].some(key => Object.hasOwn(extra.preferences, key))) throw new Error('协议模式、计价和参数策略请使用对应表单，避免草稿互相覆盖。');
    const channel = { ...extra, ...connection({ validate, forCapabilities }), groups: parseGroups(form.elements.channel_groups.value), preferences: { ...(extra.preferences || {}), protocol_mode: form.elements.protocol_mode.value } };
    channel.preferences.allow_byok = form.elements.allow_byok.checked;
    channel.preferences.response_policy = { ...(config.preferences?.response_policy || {}), retry_empty: form.elements.retry_empty.checked };
    channel.preferences.reasoning_history = form.elements.reasoning_history.value;
    channel.preferences.reasoning_suffixes = form.elements.reasoning_suffixes.checked ? config.preferences?.reasoning_suffixes && Object.keys(config.preferences.reasoning_suffixes).length ? config.preferences.reasoning_suffixes : { '-high': { effort: 'high', budget_tokens: 2048 }, '-thinking': { effort: 'high', budget_tokens: 2048 } } : {};
    if (!forCapabilities) {
      let policy;
      try { policy = parametersEditor.value(); } catch (error) { activateTab('parameters'); throw error; }
      if (policy !== undefined) channel.preferences.parameter_policy = policy;
      let pricing;
      try { pricing = modelsEditor.pricing(); } catch (error) { activateTab('models'); throw error; }
      if (pricing !== undefined || config.preferences?.usage_pricing !== undefined) channel.preferences.usage_pricing = pricing || {};
      if (model !== undefined) channel.model = model;
      else {
        const rows = modelsEditor.snapshot();
        if (rows.length) channel.model = rowsToModels(rows);
      }
    }
    return channel;
  }
  function dto(channel, request) { return { ...(existing ? { provider: existing.provider } : {}), channel, ...(request ? { request } : {}) }; }
  function markStale() {
    if (!form || closed) return;
    if (previewExists) form.querySelector('.channel-preview-stale').textContent = '草稿已变更。下方仍是上一次构造结果，请重新预览。';
  }
  function updatePreviewModels() {
    if (!form || !modelsEditor) return;
    const control = form.elements.preview_model, previous = control.value;
    const names = [...new Set(modelsEditor.snapshot().map(row => row.alias.trim() || row.upstream.trim()).filter(Boolean))];
    control.innerHTML = names.length ? names.map(name => `<option value="${esc(name)}">${esc(name)}</option>`).join('') : '<option value="">请先在模型页添加模型</option>';
    if (names.includes(previous)) control.value = previous;
    markStale();
  }
  function activateTab(name, focus = false) {
    form.querySelectorAll('[data-channel-tab]').forEach(tab => { const active = tab.dataset.channelTab === name; tab.setAttribute('aria-selected', String(active)); tab.tabIndex = active ? 0 : -1; if (active && focus) tab.focus(); });
    form.querySelectorAll('[data-channel-panel]').forEach(panel => { panel.hidden = panel.dataset.channelPanel !== name; });
    form.querySelector('#channel-preview').hidden = name === 'models';
    if (name !== 'models') updatePreviewModels();
    form.querySelector('.dialog-body').scrollTop = 0;
  }
  async function refreshCapabilities() {
    const sequence = ++schemaSequence; schemaController?.abort(); schemaController = new AbortController();
    const button = form.querySelector('[data-channel-action="preview"]'); button.disabled = true;
    const status = form.querySelector('.channel-preview-capability'); status.textContent = '正在读取当前端点能力…';
    try {
      const endpoint = form.elements.preview_endpoint.value;
      const result = await api.request('/channels/parameter-schema', { method: 'POST', body: dto(draftChannel({ forCapabilities: true }), endpoint ? { endpoint } : undefined), signal: schemaController.signal });
      if (closed || sequence !== schemaSequence) return;
      capabilities = result.capabilities;
      parametersEditor.setCapabilities(capabilities);
      const endpoints = array(capabilities?.endpoints);
      const selector = form.elements.preview_endpoint;
      selector.innerHTML = endpoints.length ? endpoints.map(path => `<option value="${esc(path)}">${esc(endpointLabel(path))} · ${esc(path)}</option>`).join('') : '<option value="">当前类型没有可预览端点</option>';
      selector.value = endpoints.includes(endpoint) ? endpoint : endpoints[0] || '';
      if (selector.value && selector.value !== endpoint) { if (!sampleEdited) form.elements.preview_body.value = JSON.stringify(sampleBody(selector.value), null, 2); await refreshCapabilities(); return; }
      const supported = canPreview(capabilities, selector.value);
      button.disabled = !supported;
      status.textContent = capabilities?.plugin_envelope ? '宿主执行信封，不是插件内部最终请求；无法由此推断插件内部的上游 URL 或认证。' : supported ? '由后端真实请求构造器解析最终接口和参数，不由前端拼接 URL。' : '当前类型 / 端点不支持准确预览。已有配置仍可编辑保存，不会展示伪造的最终请求。';
    } catch (error) {
      if (closed || sequence !== schemaSequence) return;
      capabilities = null; status.textContent = `参数能力读取失败：${error.message}`; parametersEditor.setCapabilityError(error);
    }
  }
  function renderPreview(result) {
    const headers = result.headers;
    const trace = result.policy_trace;
    const warnings = result.policy_report.warnings;
    const output = form.querySelector('.channel-preview-result');
    output.innerHTML = `<div class="preview-scope ${result.scope === 'plugin_envelope' ? 'is-plugin' : ''}"><strong>${result.scope === 'plugin_envelope' ? '宿主执行信封，不是插件内部最终请求' : '最终上游请求（未发送）'}</strong><span>${esc(result.method || '—')}</span><code>${esc(result.url || '未返回 URL')}</code></div><div class="preview-model-names"><span>对外模型：<strong>${esc(result.requested_model ?? '—')}</strong></span><span>上游模型：<strong>${esc(result.upstream_model ?? '—')}</strong></span></div>${warnings.length ? notice(warnings.map(value => typeof value === 'string' ? value : JSON.stringify(value)).join('；'), 'warning') : ''}<details class="advanced" open><summary>脱敏请求头</summary>${Object.keys(headers).length ? `<dl class="preview-headers">${Object.entries(headers).map(([name, value]) => `<div><dt>${esc(name)}</dt><dd><code>${esc(value)}</code></dd></div>`).join('')}</dl>` : '<p class="hint">没有返回额外请求头。</p>'}</details>${jsonDetails(result.body, '实际构造的请求体（脱敏）')}<h4>参数来源与约束处理</h4>${trace.length ? `<div class="table-scroll"><table><thead><tr><th>字段路径</th><th>来源</th><th>处理</th></tr></thead><tbody>${trace.map(row => `<tr><td><code>${esc(row.path)}</code></td><td>${esc(row.source)}</td><td>${esc(row.action)}</td></tr>`).join('')}</tbody></table></div>` : '<p class="hint">后端未返回参数修改记录。</p>'}`;
    output.hidden = false; previewExists = true; form.querySelector('.channel-preview-stale').textContent = '仅构造请求，不发送、不保存。修改草稿后请重新预览。';
  }
  async function preview() {
    const sequence = ++previewSequence; previewController?.abort(); previewController = new AbortController();
    const button = form.querySelector('[data-channel-action="preview"]'); button.disabled = true;
    const error = form.querySelector('.channel-preview-error'); error.hidden = true;
    try {
      const model = form.elements.preview_model.value;
      if (!model) throw new Error('请先在模型页添加一个模型，再选择要预览的公开模型。');
      const body = parseJSON(form.elements.preview_body.value, '示例请求体');
      const result = await api.request('/channels/preview', { method: 'POST', body: dto(draftChannel(), { endpoint: form.elements.preview_endpoint.value, model, stream: form.elements.preview_stream.checked, body }), signal: previewController.signal });
      if (closed || sequence !== previewSequence) return;
      renderPreview(result);
    } catch (reason) {
      if (closed || sequence !== previewSequence) return;
      error.textContent = reason.message; error.hidden = false;
    } finally { if (!closed && sequence === previewSequence) button.disabled = !canPreview(capabilities, form.elements.preview_endpoint.value); }
  }

  form = dialog({ title: existing ? `编辑渠道 · ${existing.provider}` : '添加渠道', size: 'channel', submitLabel: existing ? '保存渠道' : '创建渠道',
    body: `<div class="channel-tabs" role="tablist" aria-label="渠道编辑页面">${[['basic', '基础与账号', 'channels'], ['models', '模型与映射', 'box'], ['parameters', '高级参数', 'filter']].map(([key, label, name], index) => `<button type="button" role="tab" id="channel-tab-${key}" data-channel-tab="${key}" aria-controls="channel-panel-${key}" aria-selected="${index === 0}" tabindex="${index === 0 ? 0 : -1}">${icon(name)}${label}</button>`).join('')}</div>` +
      `<section data-channel-panel="basic" id="channel-panel-basic" role="tabpanel" aria-labelledby="channel-tab-basic">${pluginsResult.status === 'rejected' ? notice(`无法读取渠道类型：${pluginsResult.reason.message}`, 'warning') : ''}<div class="channel-basic-grid"><div>${field('provider', '渠道名称', existing?.provider || '', { required: true, readonly: Boolean(existing), placeholder: '例如：codex-main' })}${select('channel_type', '渠道类型', options.length ? options : [{ value: '', label: '暂无可用类型' }], selected)}</div><div id="channel-builtin-fields"><div id="channel-oauth-helper" class="field" hidden><span class="field-label">认证方式</span><div class="segmented-tabs" role="tablist" aria-label="账号认证方式" data-auth-mode-tabs><button type="button" class="button small primary" data-auth-mode="oauth" aria-pressed="true">${icon('key')}OAuth 登录</button><button type="button" class="button small" data-auth-mode="file" aria-pressed="false">${icon('upload')}导入文件</button><button type="button" class="button small" data-auth-mode="apikey" aria-pressed="false">${icon('edit')}手动填写</button></div><div data-auth-pane="oauth"><div class="inline-row"><button type="button" class="button primary" data-channel-oauth-start>${icon('key')}打开浏览器 OAuth 授权</button></div><div data-channel-oauth-flow hidden><label class="field"><span>授权链接</span><div class="inline-row"><input type="url" data-channel-oauth-url readonly><button type="button" class="button small" data-channel-oauth-copy>${icon('copy')}复制</button></div></label><label class="field"><span data-channel-oauth-step2-label>回调地址或授权码</span><div class="inline-row"><input type="text" data-channel-oauth-callback placeholder="粘贴回调 URL 或授权码 (code#state)" autocomplete="off" spellcheck="false"><button type="button" class="button primary" data-channel-oauth-exchange>${icon('check')}验证登录</button></div></label></div></div><div data-auth-pane="file" hidden><div class="inline-row"><label class="button primary" for="channel-auth-file-input">${icon('upload')}选择 auth.json 或凭据文件</label><input id="channel-auth-file-input" class="file-picker" type="file" data-channel-auth-file accept=".json,application/json"></div></div><div class="notice success" data-channel-account-card hidden></div><p class="hint" data-channel-auth-status></p></div><div id="channel-manual-cred-fields">${field('base_url', '接口地址 (Base URL)', initialURL, { type: 'url', placeholder: 'https://api.example.com/v1' })}${field('api', 'Token / API Key', '', { type: 'password', placeholder: existing ? '留空保留原凭据' : '粘贴 Token 或 API Key' })}</div></div></div><div class="field"><label class="field-label" for="f-channel_groups_input">分组</label><input id="f-channel_groups" name="channel_groups" type="hidden" value="${esc(initialGroups.join(', '))}"><div class="group-picker"><div class="group-chip-list" data-channel-group-chips></div><input id="f-channel_groups_input" type="text" class="group-inline-input" data-channel-new-group placeholder="+ 输入新分组按回车" aria-label="输入新分组名"></div></div>${select('protocol_mode', '协议处理', [{ value: 'auto', label: '兼容适配' }, { value: 'native', label: '原生透传' }], config.preferences?.protocol_mode || 'auto')}<p id="protocol-mode-hint" class="hint" hidden></p><details class="advanced"><summary>路由与思考策略</summary>${checkbox('allow_byok', '允许调用方自带上游 Key', config.preferences?.allow_byok === true)}${checkbox('retry_empty', '空响应自动切换渠道', config.preferences?.response_policy?.retry_empty === true)}${select('reasoning_history', '多轮思考历史', [{ value: 'preserve', label: '保留思考块' }, { value: 'strip', label: '剥离历史思考块' }], config.preferences?.reasoning_history || 'preserve')}${checkbox('reasoning_suffixes', '提供 -high / -thinking 模型别名', Object.keys(config.preferences?.reasoning_suffixes || {}).length > 0)}</details><div id="channel-schema"></div><p id="channel-type-hint" class="hint" role="status" hidden></p><details class="advanced"><summary>附加配置 JSON</summary>${field('extra', '附加配置 JSON', '{}', { type: 'textarea' })}</details></section>` +
      `<section data-channel-panel="models" id="channel-panel-models" role="tabpanel" aria-labelledby="channel-tab-models" hidden><div class="channel-models" id="channel-models"></div></section><section data-channel-panel="parameters" id="channel-panel-parameters" role="tabpanel" aria-labelledby="channel-tab-parameters" hidden><div id="channel-parameters"></div></section>` +
      `<section id="channel-preview" class="channel-preview"><div class="parameter-section-heading"><div><h3>请求预览</h3></div><button type="button" class="button" data-channel-action="refresh-schema">${icon('refresh')}刷新能力</button></div><div class="preview-request-controls">${select('preview_endpoint', '请求端点', initialEndpoints.length ? initialEndpoints.map(path => ({ value: path, label: `${endpointLabel(path)} · ${path}` })) : [{ value: '', label: '正在读取可用端点' }], initialEndpoint)}${select('preview_model', '公开模型 / 别名', [{ value: '', label: '请先在模型页添加模型' }])}</div>${checkbox('preview_stream', '流式请求预览')}<p class="channel-preview-capability hint" role="status"></p><details class="advanced"><summary>示例请求体</summary>${field('preview_body', '示例 Payload', JSON.stringify(sampleBody(initialEndpoint), null, 2), { type: 'textarea', rows: 5 })}</details><div class="preview-actions"><button type="button" class="button primary" data-channel-action="preview" disabled>${icon('eye')}构造预览</button><span class="channel-preview-stale hint" role="status"></span></div><div class="notice danger channel-preview-error" role="alert" hidden></div><div class="channel-preview-result" hidden></div></section>`,
    onMount: mounted => {
      form = mounted;
      form.noValidate = true;
      form.insertBefore(form.querySelector('.channel-tabs'), form.querySelector('.dialog-body'));
      form.elements.channel_type.required = true; if (savedPlugin) form.elements.channel_type.disabled = true;
      const channelMark = document.createElement('span');
      channelMark.className = 'channel-ai-mark';
      form.querySelector('label[for="f-channel_type"]').prepend(channelMark);
      const updateBrand = () => {
        const type = getType();
        channelMark.innerHTML = aiIcon(channelBrand({ provider: form.elements.provider.value, base_url: type?.kind === 'service' ? '' : form.elements.base_url.value, engine: type?.plugin?.manifest?.engine, channel_plugin: type?.plugin?.manifest?.name }), 'channels');
      };
      form.elements.base_url.addEventListener('change', updateBrand);
      form.elements.provider.addEventListener('change', updateBrand);
      parametersEditor = createParametersEditor(form.querySelector('#channel-parameters'), { policy: config.preferences?.parameter_policy, onChange: markStale });
      form.elements.protocol_mode.addEventListener('change', () => { markStale(); refreshCapabilities(); });
      form.elements.base_url.addEventListener('input', () => { urlEdited = true; markStale(); });
      const updateType = () => {
        const nativeSupported = ['builtin:gpt', 'builtin:codex', 'builtin:antigravity', 'builtin:openrouter', 'builtin:claude', 'builtin:gemini', 'builtin:vertex', 'builtin:vertex-gemini'].includes(form.elements.channel_type.value);
        form.elements.protocol_mode.querySelector('option[value="native"]').disabled = !nativeSupported;
        form.elements.protocol_mode.disabled = !nativeSupported;
        if (!nativeSupported) form.elements.protocol_mode.value = 'auto';
        selected = form.elements.channel_type.value;
        const type = getType(), service = type?.kind === 'service', schemaRoot = form.querySelector('#channel-schema');
        if (schemaRoot.firstElementChild) schemaDrafts.set(previousType, schemaRoot.firstElementChild);
        schema = service ? type?.plugin?.manifest?.config_schema || [] : [];
        form.querySelector('#channel-builtin-fields').hidden = Boolean(service);
        form.elements.base_url.disabled = Boolean(service); form.elements.api.disabled = Boolean(service); form.elements.base_url.required = !service;
        schemaRoot.hidden = !service;
        if (service) {
          let fields = schemaDrafts.get(selected);
          if (!fields) { fields = document.createElement('div'); fields.className = 'schema-fields'; fields.innerHTML = `<h3>插件连接配置</h3>${schemaFields(schema, { ...(type?.plugin?.config || {}), ...(selected === `service:${savedPlugin}` ? savedPluginConfig : {}) })}`; }
          schemaRoot.replaceChildren(fields);
        } else { schemaRoot.replaceChildren(); if (!urlEdited) form.elements.base_url.value = type?.plugin?.manifest?.default_base_url || ''; }
        const oauthHelper = form.querySelector('#channel-oauth-helper');
        const isOAuthChannel = ['builtin:codex', 'builtin:claude', 'builtin:antigravity'].includes(selected);
        if (oauthHelper) oauthHelper.hidden = !isOAuthChannel;
        syncAuthModeUI();
        const changed = previousType !== selected;
        const hint = form.querySelector('#channel-type-hint'); hint.hidden = true;
        if (changed) {
          modelsEditor?.resetCandidates();
          const endpoints = array(type?.plugin?.manifest?.capabilities?.endpoints);
          form.elements.preview_endpoint.innerHTML = endpoints.map(path => `<option value="${esc(path)}">${esc(endpointLabel(path))} · ${esc(path)}</option>`).join('');
          if (!sampleEdited) form.elements.preview_body.value = JSON.stringify(sampleBody(form.elements.preview_endpoint.value), null, 2);
        }
        updateBrand(); previousType = selected; markStale();
      };
      const manualCredFields = form.querySelector('#channel-manual-cred-fields');
      function syncAuthModeUI() {
        const isOAuthChannel = ['builtin:codex', 'builtin:claude', 'builtin:antigravity'].includes(selected);
        form.querySelectorAll('[data-auth-mode]').forEach(btn => {
          const active = btn.dataset.authMode === currentAuthMode;
          btn.classList.toggle('primary', active);
          btn.setAttribute('aria-pressed', String(active));
        });
        form.querySelectorAll('[data-auth-pane]').forEach(pane => {
          pane.hidden = !isOAuthChannel || pane.dataset.authPane !== currentAuthMode;
        });
        if (manualCredFields) {
          manualCredFields.hidden = isOAuthChannel && currentAuthMode !== 'apikey';
        }
      }
      form.querySelector('[data-auth-mode-tabs]')?.addEventListener('click', event => {
        const btn = event.target.closest('[data-auth-mode]');
        if (!btn) return;
        currentAuthMode = btn.dataset.authMode;
        syncAuthModeUI();
      });
      form.elements.channel_type.addEventListener('change', () => { updateType(); refreshCapabilities(); }); updateType();
      const statusEl = form.querySelector('[data-channel-auth-status]');
      const accountCard = form.querySelector('[data-channel-account-card]');
      const oauthFlow = form.querySelector('[data-channel-oauth-flow]');
      function applyLoggedAccount({ credential, email, plan, accountId }) {
        form.elements.api.value = credential || '';
        const engine = selected.slice('builtin:'.length) || 'codex';
        if (!existing && !form.elements.provider.value.trim()) {
          const prefix = email ? email.split('@')[0].replace(/[^a-zA-Z0-9_-]/g, '') : '';
          form.elements.provider.value = prefix ? `${engine}-${prefix}` : `${engine}-account`;
          updateBrand();
        }
        const parts = [email && `账号邮箱：${email}`, plan && `订阅套餐：${String(plan).toUpperCase()}`, accountId && `ID：${accountId}`].filter(Boolean);
        if (accountCard) {
          accountCard.textContent = parts.length ? `已登录授权（${parts.join(' · ')}）— 直接点击右下角「${existing ? '保存渠道' : '创建渠道'}」即可生效！` : '已获取账号授权 Token — 直接点击右下角保存即可生效！';
          accountCard.hidden = false;
        }
        if (statusEl) statusEl.textContent = '账号凭据已就绪。';
        if (oauthFlow) oauthFlow.hidden = true;
        markStale();
      }
      form.querySelector('[data-channel-oauth-copy]')?.addEventListener('click', async () => {
        const urlVal = form.querySelector('[data-channel-oauth-url]')?.value || '';
        if (!urlVal) return;
        try { await navigator.clipboard.writeText(urlVal); toast('授权链接已复制'); } catch {}
      });
      form.querySelector('[data-channel-oauth-start]')?.addEventListener('click', async () => {
        try {
          clearInterval(oauthPollTimer);
          const engine = selected.slice('builtin:'.length) || 'codex';
          oauthSession = await api.get(`/channels/keys?${new URLSearchParams({ oauth: 'authorize', provider: engine })}`);
          if (!oauthFlow) return;
          form.querySelector('[data-channel-oauth-url]').value = oauthSession.authorize_url;
          const step2Label = form.querySelector('[data-channel-oauth-step2-label]');
          if (step2Label) {
            step2Label.textContent = engine === 'claude' ? '粘贴 Claude 授权码 (code#state)' : '本机自动捕获回调，也可手动粘贴回调 URL';
          }
          oauthFlow.hidden = false;
          window.open(oauthSession.authorize_url, '_blank', 'noopener');
          form.querySelector('[data-channel-oauth-callback]')?.focus();
          if (statusEl) statusEl.textContent = engine === 'claude' ? '等待粘贴授权码…' : '等待浏览器授权…';
          if (oauthSession.auto_callback && oauthSession.state) {
            oauthPollTimer = setInterval(async () => {
              if (closed || !oauthSession?.state) { clearInterval(oauthPollTimer); return; }
              try {
                const st = await api.get(`/channels/keys?${new URLSearchParams({ oauth: 'status', state: oauthSession.state })}`);
                if (st.status === 'completed' && st.credential) {
                  clearInterval(oauthPollTimer);
                  applyLoggedAccount({ credential: st.credential, email: st.oauth?.email, plan: st.oauth?.plan_type, accountId: st.oauth?.account_id });
                } else if (st.status === 'failed' && st.error) {
                  clearInterval(oauthPollTimer);
                  if (statusEl) statusEl.textContent = `自动回调换取 Token 失败：${st.error}`;
                }
              } catch {}
            }, 1500);
          }
        } catch (err) {
          if (statusEl) statusEl.textContent = `获取授权链接失败：${err.message}`;
        }
      });
      form.querySelector('[data-channel-oauth-exchange]')?.addEventListener('click', async () => {
        const callbackInput = form.querySelector('[data-channel-oauth-callback]');
        const code = callbackInput?.value.trim() || '';
        if (!code) { if (statusEl) statusEl.textContent = '请先粘贴登录回调 URL 或授权码。'; return; }
        try {
          clearInterval(oauthPollTimer);
          const engine = selected.slice('builtin:'.length) || 'codex';
          if (statusEl) statusEl.textContent = '正在向认证服务换取 Token 并解析账号邮箱…';
          const res = await api.post('/channels/keys', {
            oauth_only: true,
            oauth_exchange: {
              provider: engine,
              code,
              state: oauthSession?.state || undefined,
              code_verifier: oauthSession?.code_verifier || '',
              redirect_uri: oauthSession?.redirect_uri || undefined,
            },
          });
          if (callbackInput) callbackInput.value = '';
          applyLoggedAccount({ credential: res.credential, email: res.oauth?.email, plan: res.oauth?.plan_type, accountId: res.oauth?.account_id });
        } catch (err) {
          if (statusEl) statusEl.textContent = `登录换取 Token 失败：${err.message}`;
        }
      });
      const authFileInput = form.querySelector('[data-channel-auth-file]');
      authFileInput?.addEventListener('change', async () => {
        const file = authFileInput.files?.[0];
        if (!file || !statusEl) return;
        try {
          const engine = selected.slice('builtin:'.length);
          const parsed = extractAccountFromJson(await file.text(), engine);
          applyLoggedAccount({ credential: parsed.credential, email: parsed.email, plan: parsed.plan, accountId: parsed.accountId });
        } catch (err) {
          statusEl.textContent = `解析失败：${err.message}`;
        }
      });
      const chipsRoot = form.querySelector('[data-channel-group-chips]');
      const newGroupInput = form.querySelector('[data-channel-new-group]');
      let keysRevision = keysData.revision || '';
      const renderGroupChips = () => {
        if (!chipsRoot) return;
        const activeGroups = parseGroups(form.elements.channel_groups.value);
        chipsRoot.innerHTML = knownGroups.map(name => `<button class="group-chip" type="button" data-channel-group-chip="${esc(name)}" aria-pressed="${activeGroups.includes(name)}">${esc(name)}</button>`).join('');
      };
      renderGroupChips();
      chipsRoot?.addEventListener('click', event => {
        const chip = event.target.closest('[data-channel-group-chip]');
        if (!chip) return;
        const targetGroup = chip.dataset.channelGroupChip;
        const values = parseGroups(form.elements.channel_groups.value);
        const next = values.includes(targetGroup) ? values.filter(v => v !== targetGroup) : [...values, targetGroup];
        form.elements.channel_groups.value = (next.length ? next : ['default']).join(', ');
        renderGroupChips(); markStale();
      });
      const addChannelGroup = async () => {
        const rawInput = newGroupInput?.value || '';
        const added = parseGroups(rawInput);
        if (!added.length) return;
        const current = parseGroups(form.elements.channel_groups.value);
        for (const item of added) {
          if (!knownGroups.includes(item)) knownGroups.push(item);
          if (!current.includes(item)) current.push(item);
        }
        knownGroups.sort();
        form.elements.channel_groups.value = current.join(', ');
        if (newGroupInput) newGroupInput.value = '';
        renderGroupChips(); markStale();
        if (keysRevision) {
          try {
            const saved = await api.patch('/api-keys', { revision: keysRevision, group_action: 'add', group_name: added.join(', ') });
            if (saved?.revision) keysRevision = saved.revision;
          } catch {}
        }
      };
      newGroupInput?.addEventListener('keydown', event => {
        if ((event.key === 'Enter' || event.key === ',') && !event.isComposing) {
          event.preventDefault();
          addChannelGroup();
        }
      });
      newGroupInput?.addEventListener('blur', () => { if (newGroupInput.value.trim()) addChannelGroup(); });
      const initialModelsList = config.model || existing?.models || [];
      modelsEditor = createModelsEditor(form.querySelector('#channel-models'), {
        models: initialModelsList,
        pricing: config.preferences?.usage_pricing || {},
        onChange: updatePreviewModels,
        onPricingChange: markStale,
        discover: signal => api.request('/channels/models', { method: 'POST', body: dto(connection()), signal }),
        testModel: (modelName, opts = {}) => {
          const request = { model: modelName, messages: [{ role: 'user', content: opts.prompt || 'hi' }], stream: false };
          if (opts.limit !== '' && opts.limit !== undefined) request.max_tokens = Number(opts.limit);
          if (opts.temperature !== '' && opts.temperature !== undefined && Number.isFinite(Number(opts.temperature))) request.temperature = Number(opts.temperature);
          if (opts.topP !== '' && opts.topP !== undefined && Number.isFinite(Number(opts.topP))) request.top_p = Number(opts.topP);
          if (opts.reasoningEffort) request.reasoning_effort = opts.reasoningEffort;
          return api.request('/test', { method: 'POST', timeoutMs: 65000, body: { direct: true, provider: existing?.provider || '', model: modelName, channel: draftChannel(), request } });
        },
      });
      updatePreviewModels();
      form.querySelectorAll('[data-channel-tab]').forEach(tab => tab.addEventListener('click', () => activateTab(tab.dataset.channelTab)));
      form.querySelector('.channel-tabs').addEventListener('keydown', event => {
        const tabs = [...form.querySelectorAll('[data-channel-tab]')], index = tabs.indexOf(document.activeElement);
        if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key) || index < 0) return;
        event.preventDefault(); const next = event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : (index + (event.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length;
        activateTab(tabs[next].dataset.channelTab, true);
      });
      form.addEventListener('invalid', event => { const panel = event.target.closest('[data-channel-panel]'); if (panel?.hidden) activateTab(panel.dataset.channelPanel); }, true);
      form.addEventListener('input', event => { if (event.target.closest('#channel-models')) return; if (event.target.name === 'preview_body') sampleEdited = true; markStale(); });
      form.elements.preview_endpoint.addEventListener('change', () => { if (!sampleEdited) form.elements.preview_body.value = JSON.stringify(sampleBody(form.elements.preview_endpoint.value), null, 2); markStale(); refreshCapabilities(); });
      form.elements.preview_model.addEventListener('change', markStale); form.elements.preview_stream.addEventListener('change', markStale);
      form.querySelector('[data-channel-action="refresh-schema"]').addEventListener('click', refreshCapabilities);
      form.querySelector('[data-channel-action="preview"]').addEventListener('click', preview);
      activateTab('basic'); refreshCapabilities();
    },
    onClose: () => { closed = true; clearInterval(oauthPollTimer); schemaSequence++; previewSequence++; schemaController?.abort(); previewController?.abort(); modelsEditor?.destroy(); parametersEditor?.destroy(); },
    onSubmit: async () => {
      let model;
      try { model = modelsEditor.value(); } catch (error) { activateTab('models'); throw error; }
      const channel = draftChannel({ validate: true, model });
      if (existing) await api.patch('/channels', { revision: currentRevision, provider: existing.provider, patch: channel });
      else await api.post('/channels', { revision: currentRevision, channel: { ...channel, provider: form.elements.provider.value.trim() } });
      toast(existing ? '渠道已保存并热生效' : '渠道已创建并热生效'); await onSaved();
    },
  });
}
