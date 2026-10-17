import { mountRequestLogs } from './request-logs.js';
import { mountWorkspace, confirmWorkspaceLeave } from './workspace.js';
import { mountBackendLogs } from './backend-logs.js';
import { mountTesting, openChannelTestDialog } from './model-testing.js';
import { mountPlayground } from './playground.js';
import { mountKeys, openGroupManager } from './key-manager.js';
import { mountLogSettings, settingsNav, logsNav } from './log-settings.js';
import { usageDashboard, usageBreakdown, usageTrend, openUsageDetail } from './usage-dashboard.js';
import { pluginList, openPluginUpload, openPluginLogs } from './plugin-manager.js';
import { openChannelEditor } from './channel-editor.js';
import { openChannelAccounts } from './channel-accounts.js';
import { balanceSummary, openChannelBalance } from './channel-balances.js';
import * as api from './api.js';
import { icon, mountIcons, aiIcon, channelBrand } from './icons.js';
import { esc, fmt, stamp, badge, notice, empty, failure, loading, jsonDetails, field, select, checkbox, parseJSON, toast, dialog, confirmAction } from './ui.js';

const main = document.querySelector('#main-content');
const routes = {
  overview: ['概览'],
  channels: ['渠道与模型'],
  plugins: ['插件管理'],
  interceptors: ['请求拦截器'],
  keys: ['API Key'],
  workspace: ['工作区'],
  testing: ['模型测试'],
  playground: ['对话调试台'],
  'backend-logs': ['后台日志'],
  logs: ['请求日志'],
  'log-settings': ['日志与隐私'],
  usage: ['用量与管理审计'],
  settings: ['系统设置'],
};
let currentRoute = 'overview';
let controller;
let generation = 0;
let state = {};
let authenticated = false;
let pluginGroup = 'builtin';
let usageDays = 1;
let usageChannel = '';
let usageModel = '';
let usageGroup = 'model';
const button = (label, action, name = '', primary = false, index = '') => `<button class="button ${primary ? 'primary' : ''}" type="button" data-action="${action}" ${index !== '' ? `data-index="${index}"` : ''}>${name ? icon(name) : ''}${esc(label)}</button>`;
const mini = (label, action, index, name = '', danger = false) => `<button class="button small ${danger ? 'text-danger' : ''}" type="button" data-action="${action}" data-index="${index}">${name ? icon(name) : ''}${esc(label)}</button>`;
const head = (actions = '') => `<div class="page-heading"><h1>${routes[currentRoute][0]}</h1><div class="page-actions">${button('刷新', 'refresh', 'refresh')}${actions}</div></div>`;
const table = (heads, rows) => `<div class="table-scroll" tabindex="0" role="region" aria-label="可横向滚动的数据表"><table><thead><tr>${heads.map(h => `<th scope="col">${h}</th>`).join('')}</tr></thead><tbody>${rows}</tbody></table></div>`;
const countFooter = (count, label = '条记录') => `<div class="table-footer">共 ${fmt(count)} ${label}</div>`;
const tags = (items = [], limit = 4) => `<div class="tags">${items.map((item, idx) => { const text = typeof item === 'string' ? item : item.id || JSON.stringify(item); return `<span class="tag-copy-item"${idx >= limit ? ' hidden data-extra-tag' : ''} data-copy-text="${esc(text)}" title="点击复制">${badge(text)}</span>`; }).join('')}${items.length > limit ? `<button type="button" class="badge tag-expand-btn" data-action="expand-tags">+${items.length - limit}</button>` : ''}</div>`;
const read = path => api.get(path, { signal: controller.signal });
const scalar = value => value === null || value === undefined ? '—' : typeof value === 'object' ? JSON.stringify(value) : String(value);
const metric = (label, value, note, name) => `<article class="metric-card"><div class="metric-head"><span>${esc(label)}</span><span class="metric-icon">${icon(name)}</span></div><div class="metric-value">${esc(value)}</div>${note ? `<div class="metric-note">${esc(note)}</div>` : ''}</article>`;
const definition = rows => `<dl class="definition-list">${rows.map(([key, value]) => `<div><dt>${esc(key)}</dt><dd>${value}</dd></div>`).join('')}</dl>`;
const splitLines = value => [...new Set(value.split(/\r?\n|,/).map(s => s.trim()).filter(Boolean))];

function usageQuery() {
  const params = new URLSearchParams({ days: String(usageDays) });
  if (usageModel) params.set('model', usageModel);
  if (usageChannel) params.set('channel', usageChannel);
  return params;
}
const usageFilters = () => ({ days: usageDays, model: usageModel, channel: usageChannel, group: usageGroup });
async function openUpdateDialog() {
  const modal = document.querySelector('#editor-dialog'), content = document.querySelector('#dialog-content');
  content.innerHTML = `<div class="dialog-header"><h2 id="dialog-title">版本检查与更新</h2><button class="icon-button" type="button" id="dialog-close" aria-label="关闭">${icon('close')}</button></div><div class="dialog-body">${loading('正在检查 GitHub 最新版本…')}</div>`;
  content.querySelector('#dialog-close').onclick = () => modal.close();
  if (!modal.open) modal.showModal();
  try {
    const info = await api.get('/update');
    const shortCur = (info.current_commit || 'unknown').slice(0, 8);
    const shortLatest = (info.latest_commit || '—').slice(0, 8);
    const statusBadge = info.has_update ? badge('发现新版本', 'orange') : info.latest_commit ? badge('已是最新', 'green') : badge('离线/待检查', 'subtle');
    const dockerCmd = info.docker_command || 'docker compose pull && docker compose up -d';
    content.innerHTML = `
      <div class="dialog-header">
        <div><h2 id="dialog-title">版本检查与更新</h2><p class="subline">仓库：${esc(info.repo || 'Mitasul/miora')} · 当前版本 v${esc(info.version || '0.1.99')}</p></div>
        <button class="icon-button" type="button" id="dialog-close" aria-label="关闭">${icon('close')}</button>
      </div>
      <div class="dialog-body">
        <div class="form-section">
          ${definition([
            ['状态', statusBadge],
            ['当前 Commit', `<code class="mono">${esc(shortCur)}</code>`],
            ['最新 Commit', `<code class="mono">${esc(shortLatest)}</code>${info.latest_date ? ` <span class="subline">(${esc(stamp(info.latest_date))})</span>` : ''}`],
            ['最新更新说明', esc(info.latest_message || info.check_error || '—')],
          ])}
        </div>
        <div class="form-section">
          <div class="form-section-header"><h2>Docker 部署秒级更新命令</h2><p>在服务器宿主机执行以下命令即可拉取 GitHub 云端预编译镜像并重启：</p></div>
          <div class="toolbar"><code class="mono">${esc(dockerCmd)}</code><button class="button small" type="button" id="copy-docker-update">${icon('copy')}复制命令</button></div>
        </div>
        <div id="update-output" hidden></div>
      </div>
      <div class="dialog-footer">
        <button class="button" type="button" id="dialog-cancel">关闭</button>
        ${info.git_available && !info.in_docker ? `<button class="button primary" type="button" id="run-git-update">${icon('upload')}一键拉取最新代码 (Git Pull)</button>` : ''}
      </div>`;
    content.querySelector('#dialog-close').onclick = () => modal.close();
    content.querySelector('#dialog-cancel').onclick = () => modal.close();
    content.querySelector('#copy-docker-update')?.addEventListener('click', async () => {
      try { await navigator.clipboard.writeText(dockerCmd); toast('已复制 Docker 更新命令'); }
      catch { toast('复制失败，请手动选中复制'); }
    });
    content.querySelector('#run-git-update')?.addEventListener('click', async event => {
      const btn = event.currentTarget;
      const outBox = content.querySelector('#update-output');
      btn.disabled = true;
      btn.innerHTML = `<span class="spinner"></span>正在拉取更新…`;
      try {
        const res = await api.post('/update', {});
        outBox.hidden = false;
        outBox.innerHTML = notice(`${res.message || ''}${res.output ? `\n${res.output}` : ''}`, res.ok ? 'success' : 'warning');
        if (res.ok) toast('代码已拉取至最新提交');
      } catch (err) {
        outBox.hidden = false;
        outBox.innerHTML = notice(err.message, 'danger');
      } finally {
        btn.disabled = false;
        btn.innerHTML = `${icon('upload')}一键拉取最新代码 (Git Pull)`;
      }
    });
  } catch (err) {
    content.innerHTML = `<div class="dialog-header"><h2 id="dialog-title">版本检查与更新</h2><button class="icon-button" type="button" id="dialog-close" aria-label="关闭">${icon('close')}</button></div><div class="dialog-body">${failure(err)}</div><div class="dialog-footer"><button class="button" type="button" id="dialog-cancel">关闭</button></div>`;
    content.querySelector('#dialog-close').onclick = () => modal.close();
    content.querySelector('#dialog-cancel').onclick = () => modal.close();
  }
}

async function overview() {
  const [data, usageResult] = await Promise.all([read('/overview'), read(`/usage?${usageQuery()}`).then(value => ({ value }), error => ({ error }))]);
  state = { overview: data, usage: usageResult.value };
  return head(button('检查更新', 'check-update', 'upload')) + (data.configuration_ready === false ? notice('网关配置未就绪，请检查渠道设置。', 'warning') : '') +
    `<div class="overview-inventory">${badge(`${fmt(data.provider_count)} 个渠道`)}${badge(`${fmt(data.model_count)} 个配置模型`)}${badge(`${fmt(data.api_key_count)} 个 API Key`)}<a href="#channels">管理渠道${icon('arrow')}</a></div>` +
    (usageResult.error ? `<section class="card">${failure(usageResult.error)}</section>` : usageDashboard(usageResult.value, usageFilters()));
}

function channelRows(channels, writable) {
  return channels.map((channel, index) => `<tr class="channel-row" data-engine="${esc(channel.channel_plugin || channel.engine || '默认')}" data-search="${esc(`${channel.provider} ${channel.base_url} ${channel.models.join(' ')}`.toLowerCase())}"><td class="channel-cell-main"><div class="channel-head-row">${writable && !channel.temporary ? `<input type="checkbox" class="channel-select-box" data-channel-select="${index}" aria-label="选择 ${esc(channel.provider)}">` : ''}<div class="table-name"><span class="row-icon ai-brand-mark">${aiIcon(channelBrand(channel), 'channels')}</span><span><strong>${esc(channel.provider)}</strong><span class="subline" title="${esc(channel.base_url)}">${esc(channel.base_url || '由渠道插件提供连接')}</span><span class="tags channel-groups">${(channel.groups || ['default']).map(group => badge(group)).join('')}</span></span></div></div></td><td class="channel-cell-type">${badge(channel.channel_plugin || channel.engine || '默认', channel.channel_plugin ? 'purple' : '')}${channel.enabled === false ? badge('已停用', 'orange') : badge('已启用', 'green')}${channel.temporary ? badge('临时', 'orange') : ''}</td><td class="channel-cell-models">${tags(channel.models, 5)}</td><td class="channel-cell-keys">${writable && !channel.temporary ? `<button type="button" class="button small ghost" data-action="channel-accounts" data-index="${index}" title="管理账号池">${icon('key')}<span class="channel-key-count">${fmt(channel.api_key_count)}</span></button>` : `<span class="channel-key-count">${fmt(channel.api_key_count)}</span>`}</td><td class="channel-cell-balance" data-channel-balance-index="${index}" data-action="quick-balance" data-index="${index}" title="点击直接刷新余额">${balanceSummary(state.balances[channel.provider])}</td><td class="channel-cell-actions"><div class="table-actions">${mini('余额', 'channel-balance', index, 'chart')}${mini('测试', 'test-channel', index, 'pulse')}${writable && !channel.temporary ? mini('账号', 'channel-accounts', index, 'key') : ''}${mini(writable && !channel.temporary ? '编辑' : '查看', 'edit-channel', index, writable && !channel.temporary ? 'edit' : 'eye')}${writable && !channel.temporary ? mini(channel.enabled === false ? '启用' : '停用', 'toggle-channel', index, channel.enabled === false ? 'play' : 'pause') : ''}${mini('统计', 'channel-usage', index, 'chart')}${writable && !channel.temporary ? mini('复制', 'copy-channel', index, 'copy') + mini('删除', 'delete-channel', index, 'trash', true) : ''}</div></td></tr>`).join('');
}
async function channels() {
  const [data, balances] = await Promise.all([read('/channels'), api.get('/channels/balances').catch(error => ({ error }))]);
  state = { channels: data, balances: balances.balances || {}, balanceError: balances.error };
  const list = data.channels;
  const filters = new URLSearchParams(window.location.hash.split('?')[1] || '');
  const activeGroup = filters.get('group') || '';
  const engines = [...new Set(list.map(channel => channel.channel_plugin || channel.engine || '默认'))].sort();
  const groups = [...new Set(['default', ...(data.groups || []), ...list.flatMap(channel => channel.groups || ['default'])])].sort();
  const groupPills = list.length ? `<div class="group-tabs" role="group" aria-label="按分组快速筛选渠道">${['', ...groups].map(g => `<button class="button small" type="button" data-action="filter-channel-group" data-channel-group-pill="${esc(g)}" aria-pressed="${g === activeGroup}">${esc(g || '全部分组')}<span class="group-count">${fmt(g ? list.filter(ch => (ch.groups || ['default']).includes(g)).length : list.length)}</span></button>`).join('')}</div>` : '';
  return head((list.length ? button('批量测试模型', 'batch-test-channels', 'pulse') : '') + (data.writable ? button('添加渠道', 'create-channel', 'plus', true) : '')) + (data.writable ? '' : notice('当前配置源只读。', 'warning')) + (state.balanceError ? notice('无法读取已保存余额：' + state.balanceError.message, 'warning') : '') + `<section class="card channels-card">${groupPills}${data.writable ? `<div class="toolbar channels-batch-bar"><button class="button small" data-action="select-channels">选择可见</button><button class="button small" data-action="enable-channels">启用选中</button><button class="button small" data-action="disable-channels">停用选中</button><button class="button small" data-action="group-channels">批量设分组</button><button class="button small" data-action="batch-test-channels">${icon('pulse')}测试选中渠道</button><button class="button small" data-action="refresh-balances">${icon('refresh')}刷新余额</button><button class="button small" data-action="manage-groups">${icon('plus')}分组管理</button></div>` : ''}<div class="toolbar channels-filter-bar"><div class="search-field">${icon('search')}<input id="channel-search" type="search" value="${esc(filters.get('q') || '')}" placeholder="搜索渠道、地址或模型…" aria-label="搜索渠道、地址或模型"></div>${select('channel_type_filter', '接入方式', [{ value: '', label: '全部接入方式' }, ...engines.map(engine => ({ value: engine, label: engine }))], filters.get('type') || '')}${select('channel_group_filter', '分组', [{ value: '', label: '全部分组' }, ...groups.map(group => ({ value: group, label: group }))], activeGroup)}<button class="button small" type="button" id="clear-channel-filters">清除筛选</button></div>${list.length ? table(['渠道名称 / 上游地址', '接入方式', '模型与映射', '账号数', '余额 / 套餐', '操作'], channelRows(list, data.writable)) : empty('还没有配置渠道', data.writable ? '点击右上角「添加渠道」开始配置。' : '当前配置源中没有渠道。', 'channels')}<div id="search-empty" hidden>${empty('没有匹配的渠道', '尝试其他渠道名称或模型关键词。', 'search')}</div><div class="table-footer" id="channel-filter-count" role="status" aria-live="polite"></div></section>`;
}
async function editChannel(index, initialType = '') {
  const pageGeneration = generation;
  const data = state.channels || await read('/channels');
  await openChannelEditor({ existing: index === undefined ? null : data.channels[index], initialType, revision: data.revision, writable: data.writable, isCurrent: () => pageGeneration === generation, onSaved: render });
}
function showChannelBalance(index) {
  const provider = state.channels.channels[index].provider;
  const pageGeneration = generation;
  openChannelBalance(provider, state.balances[provider], snapshot => {
    if (pageGeneration !== generation) return;
    state.balances[provider] = snapshot;
    const cell = main.querySelector(`[data-channel-balance-index="${index}"]`);
    if (cell) cell.innerHTML = balanceSummary(snapshot);
  });
}
async function quickRefreshBalance(index) {
  const provider = state.channels.channels[index].provider;
  const pageGeneration = generation;
  const cell = main.querySelector(`[data-channel-balance-index="${index}"]`);
  if (cell) cell.innerHTML = '<span class="hint">正在刷新余额…</span>';
  try {
    const snapshot = await api.request('/channels/balances', { method: 'POST', body: { provider }, timeoutMs: 60000 });
    if (pageGeneration !== generation) return;
    state.balances[provider] = snapshot;
    if (cell) cell.innerHTML = balanceSummary(snapshot);
    toast(`${provider} 余额已更新`);
  } catch (err) {
    if (pageGeneration !== generation) return;
    if (cell) cell.innerHTML = balanceSummary(state.balances[provider]);
    toast(`${provider} 余额获取失败：${err.message}`);
  }
}
async function batchRefreshBalances() {
  const checked = new Set(selectedChannels());
  const all = state.channels?.channels || [];
  const targets = checked.size ? all.map((ch, idx) => [ch, idx]).filter(([ch]) => checked.has(ch.provider)) : all.map((ch, idx) => [ch, idx]).filter(([ch]) => ch.enabled !== false);
  if (!targets.length) { toast('没有可刷新余额的渠道'); return; }
  toast(`正在并发刷新 ${targets.length} 个渠道余额…`);
  await Promise.allSettled(targets.map(([, idx]) => quickRefreshBalance(idx)));
}
function selectedChannels() {
  return [...main.querySelectorAll('[data-channel-select]:checked')].filter(box => !box.closest('tr').hidden).map(box => state.channels.channels[Number(box.dataset.channelSelect)].provider);
}
async function mutateChannels(providers, patch) {
  if (!providers.length) { toast('先选择渠道'); return; }
  await api.patch('/channels', { revision: state.channels.revision, providers, patch });
  toast('已保存'); await render();
}
function batchTestChannels() {
  const checked = new Set(selectedChannels());
  const all = state.channels?.channels || [];
  const targets = checked.size ? all.filter(ch => checked.has(ch.provider)) : all.filter(ch => ch.enabled !== false);
  if (!targets.length) { toast('没有可测试的渠道'); return; }
  openChannelTestDialog(targets, { revision: state.channels?.revision, writable: state.channels?.writable, onUpdated: render });
}
function groupChannels() {
  const providers = selectedChannels();
  if (!providers.length) { return openGroupManager({ onChanged: render }); }
  dialog({ title: `设置 ${providers.length} 个渠道的分组`, submitLabel: '保存', body: field('batch_groups', '分组（逗号分隔，可直接填新分组名）', 'default', { required: true }), onSubmit: async form => {
    const groups = form.elements.batch_groups.value.split(/[,，]/).map(value => value.trim()).filter(Boolean);
    await mutateChannels(providers, { groups });
  } });
}
function copyChannel(index) {
  const channel = state.channels.channels[index];
  const revision = state.channels.revision;
  dialog({ title: '复制渠道', submitLabel: '创建副本', body: field('copy_name', '渠道名称', `${channel.provider}-副本`, { required: true }), onSubmit: async form => {
    await api.post('/channels', { revision, copy_from: channel.provider, channel: { provider: form.elements.copy_name.value.trim() } });
    toast('渠道已复制'); await render();
  } });
}
function deleteChannel(index) {
  const channel = state.channels.channels[index]; const revision = state.channels.revision;
  confirmAction({ title: '删除渠道', message: `确认从源配置中删除“${channel.provider}”？该渠道上的模型将不再参与新的请求路由。此操作不可直接撤销。`, label: '确认删除', action: async () => { await api.remove('/channels', { revision, provider: channel.provider }); toast('渠道已删除'); await render(); } });
}

async function plugins() {
  const data = await read('/plugins'); state = { plugins: data };
  return head('<a class="button" href="/admin/api/plugins/template" download="example.miora.json">下载模板</a><a class="button" href="/admin/api/plugins/sdk" download="miora_plugin.py">Python SDK</a>' + button('上传插件', 'create-plugin', 'upload', true)) + pluginList(data.plugins, pluginGroup);
}
function pluginDetail(index) {
  const plugin = state.plugins.plugins[index];
  const versions = plugin.versions || [];
  dialog({ title: `版本与详情 · ${plugin.manifest?.name || plugin.id}`, submitLabel: '关闭', body: definition([['插件 ID', `<code>${esc(plugin.id)}</code>`], ['启用版本', esc(plugin.active_version || plugin.manifest?.version)], ['已安装版本', esc(plugin.installed_version || plugin.manifest?.version)], ['作者', esc(plugin.manifest?.author || '—')], ['兼容要求', esc('API ' + (plugin.manifest?.api_version || '内置') + ' / SDK ' + (plugin.manifest?.sdk_version || '内置'))], ['使用渠道', esc((plugin.channels || []).join('、') || '无')], ['更新说明', esc(plugin.manifest?.changelog || '无')], ['配置修订', fmt(plugin.revision)], ['执行端点', `<code>${esc(plugin.endpoint || '渠道插件')}</code>`]]) + `<h3 class="version-heading">执行版本</h3>${versions.length ? table(['版本 / 修订', '状态', '在途租约', '操作'], versions.map(version => `<tr><td><strong>${esc(version.version)}</strong><span class="subline">修订 ${esc(version.revision)}</span></td><td>${badge(version.active ? '当前版本' : version.draining ? '正在排空' : '历史版本', version.active ? 'green' : version.draining ? 'orange' : '')}</td><td>${fmt(version.leases)}</td><td>${plugin.kind !== 'builtin' && !version.active ? `<button class="button small" type="button" data-rollback="${esc(version.revision)}">${icon('rollback')}回滚</button>` : '—'}</td></tr>`).join('')) : empty('没有版本记录', '该插件未返回版本历史。', 'clock')}` + jsonDetails(plugin.manifest, '查看插件声明 Manifest'), onSubmit: async () => {}, onMount: form => {
    form.querySelectorAll('[data-rollback]').forEach(button => button.addEventListener('click', () => {
      const target = Number(button.dataset.rollback);
      confirmAction({ title: '确认回滚插件', message: `将“${plugin.id}”回滚到历史修订 ${target} 的配置。新的请求将使用该版本；在途请求继续排空。`, label: '确认回滚', action: async () => { await api.post(`/plugins/${encodeURIComponent(plugin.id)}/rollback`, { revision: target, expected_revision: plugin.revision }); toast('已切换到目标执行版本'); await render(); } });
    }));
  } });
}
async function togglePlugin(index) {
  const plugin = state.plugins.plugins[index];
  await api.post(`/plugins/${encodeURIComponent(plugin.id)}/${plugin.enabled ? 'disable' : 'enable'}`, { expected_revision: plugin.revision });
  toast('插件启用状态已更新'); await render();
}
async function interceptors() {
  const data = await read('/interceptors'); state = { interceptors: data };
  const list = data.interceptors;
  return head(button('添加拦截器', 'create-interceptor', 'plus', true)) + `<section class="card"><div class="card-header"><div><h2>请求处理规则</h2></div>${badge(`${list.length} 条规则`, 'purple')}</div>${list.length ? table(['顺序', '拦截器', '应用插件', '请求头规则', '状态', '操作'], list.map((item, index) => `<tr><td class="mono">${esc(item.order)}</td><td><strong>${esc(item.id)}</strong></td><td>${item.plugin_ids.length ? tags(item.plugin_ids) : badge('全部插件')}</td><td><span>设置 ${Object.keys(item.set_headers || {}).length} 项</span><span class="subline">移除 ${item.remove_headers.length} 项</span></td><td>${badge(item.enabled ? '已启用' : '已停用', item.enabled ? 'green' : 'orange')}</td><td><div class="table-actions">${mini('编辑', 'edit-interceptor', index, 'edit')}${mini(item.enabled ? '停用' : '启用', 'toggle-interceptor', index, 'power')}${mini('删除', 'delete-interceptor', index, 'trash', true)}</div></td></tr>`).join('')) : empty('没有请求拦截器', '添加一个明确的请求头规则，按顺序处理插件请求。', 'filter')}${countFooter(list.length)}</section>`;
}
function editInterceptor(index) {
  const item = index === undefined ? null : state.interceptors.interceptors[index];
  const original = state.interceptors;
  dialog({ title: item ? `编辑拦截器 · ${item.id}` : '添加请求拦截器', body: `<div class="field-row">${field('id', '规则 ID', item?.id || '', { required: true, readonly: Boolean(item), placeholder: 'request-tags' })}${field('order', '执行顺序', item?.order ?? 0, { type: 'number', required: true, step: 1 })}</div>${checkbox('enabled', '启用此规则', item?.enabled ?? true)}${field('plugin_ids', '应用插件 ID（每行一个）', (item?.plugin_ids ?? []).join('\n'), { type: 'textarea', help: '留空表示全部插件；使用插件页面显示的插件 ID。', rows: 2 })}${field('set_headers', '设置请求头（每行 name: value）', Object.entries(item?.set_headers || {}).map(([k, v]) => `${k}: ${v}`).join('\n'), { type: 'textarea', placeholder: 'x-request-source: miora', help: '只允许自定义非敏感 x-* 请求头，值中不能包含换行。' })}${field('remove_headers', '移除请求头（每行一个）', (item?.remove_headers ?? []).join('\n'), { type: 'textarea', rows: 2 })}`, onSubmit: async form => {
    const setHeaders = Object.create(null);
    for (const line of form.elements.set_headers.value.split(/\r?\n/).filter(l => l.trim())) { const colon = line.indexOf(':'); if (colon < 1) throw new Error('设置请求头应为 name: value，每行一项。'); const key = line.slice(0, colon).trim().toLowerCase(); validateHeader(key); setHeaders[key] = line.slice(colon + 1).trim(); }
    const removeHeaders = splitLines(form.elements.remove_headers.value).map(s => s.toLowerCase()); removeHeaders.forEach(validateHeader);
    const updated = { id: form.elements.id.value.trim(), enabled: form.elements.enabled.checked, order: Number(form.elements.order.value), plugin_ids: splitLines(form.elements.plugin_ids.value), set_headers: setHeaders, remove_headers: removeHeaders };
    if (!Number.isSafeInteger(updated.order)) throw new Error('执行顺序必须是整数。');
    if (original.interceptors.some((v, i) => v.id === updated.id && i !== index)) throw new Error('规则 ID 已存在。');
    const list = [...original.interceptors]; if (index === undefined) list.push(updated); else list[index] = updated;
    await api.put('/interceptors', { expected_revision: original.revision, interceptors: list }); toast('拦截器配置已保存'); await render();
  } });
}
function validateHeader(name) { if (!/^x-[a-z0-9-]+$/.test(name) || ['x-api-key', 'x-auth-token', 'x-authorization', 'x-csrf-token'].includes(name)) throw new Error(`不允许修改请求头 ${name}。仅支持自定义非敏感 x-* 请求头。`); }
async function mutateInterceptor(index, operation) {
  const original = state.interceptors; const item = original.interceptors[index];
  if (operation !== 'delete') {
    const list = original.interceptors.map(v => ({ ...v }));
    list[index].enabled = !list[index].enabled;
    await api.put('/interceptors', { expected_revision: original.revision, interceptors: list });
    toast('拦截器规则已更新'); await render();
    return;
  }
  confirmAction({ title: '删除拦截器', message: `确认删除规则“${item.id}”？新配置将影响后续插件请求。`, label: '确认更改', action: async () => { const list = original.interceptors.map(v => ({ ...v })); list.splice(index, 1); await api.put('/interceptors', { expected_revision: original.revision, interceptors: list }); toast('拦截器规则已更新'); await render(); } });
}

async function usage() {
  const pageGeneration = generation;
  const [usageResult, logsResult] = await Promise.allSettled([read(`/usage?${usageQuery()}`), read('/logs')]);
  const usageData = usageResult.status === 'fulfilled' ? usageResult.value : null;
  const logsData = logsResult.status === 'fulfilled' ? logsResult.value : null;
  if (pageGeneration !== generation) return '';
  state = { usage: usageData, logs: logsData };
  const logs = (logsData?.logs ?? []);
  return head() + logsNav('usage') + (usageResult.status === 'rejected' ? `<section class="card">${failure(usageResult.reason)}</section>` : usageDashboard(usageData, usageFilters())) + `
    <section class="card"><div class="card-header"><div><h2>管理员操作审计</h2><p>当前进程内记录 · 非模型请求日志${logsData?.persistent === false ? ' · 重启后不保留' : ''}</p></div>${badge('管理操作', '', 'shield')}</div>${logsResult.status === 'rejected' ? failure(logsResult.reason) : logs.length ? table(['发生时间', '操作', '结果'], [...logs].reverse().map(log => `<tr><td>${esc(stamp(log.timestamp))}</td><td><code>${esc(log.action)}</code></td><td>${badge(log.outcome, ['success', 'ok'].includes(log.outcome) ? 'green' : 'orange')}</td></tr>`).join('')) : empty('暂无管理员操作记录', '后续管理操作将以服务端实际记录为准显示在这里。', 'file')}${countFooter(logs.length, '条审计记录')}</section>`;
}

const settingsLabels = { model_timeout: ['模型请求超时', '秒；0 或特殊取值的含义以网关后端配置为准'], keepalive_interval: ['流式保活间隔', '秒'], cooldown_period: ['渠道冷却时间', '秒'], api_key_cooldown_period: ['上游凭据冷却时间', '秒'], api_key_rate_limit_cooldown_period: ['限流冷却时间', '秒'], api_key_quota_cooldown_period: ['配额冷却时间', '秒'] };
function resolvedSetting(data, key) { return data.resolved_fields?.[`/preferences/${key}`]?.value ?? data.effective?.preferences?.[key]; }
async function settings() {
  const data = await read('/settings'); state = { settings: data }; const readOnly = data.writable === false;
  const basicFields = Object.entries(settingsLabels).map(([key, [label, help]]) => { const value = resolvedSetting(data, key); return field(key, label, typeof value === 'number' ? value : '', { type: 'number', min: 0, step: 'any', help: typeof value === 'object' && value ? '当前为按模型配置的对象。留空保留；填写数字将改为统一值。' : `${help} · 留空保持原值` }); }).join('');
  const hedging = resolvedSetting(data, 'hedging') || {};
  return head() + settingsNav('settings') + (readOnly ? notice('远程配置为只读，请修改配置源。', 'warning') : '') +
    `<form id="settings-form" class="card"><fieldset class="settings-fields"${readOnly ? ' disabled' : ''}><section class="form-section"><div class="form-section-header"><h2>超时与冷却</h2></div><div class="field-row">${basicFields}</div></section><section class="form-section"><div class="form-section-header"><h2>并行请求</h2><p>并行请求可能增加上游调用量与费用。仅按实际需要启用。</p></div>${checkbox('hedging_enabled', '启用并行请求（Hedging）', hedging.enabled === true)}<div class="field-row">${field('max_inflight_attempts', '最大并发尝试数', hedging.max_inflight_attempts ?? '', { type: 'number', min: 1, step: 1, help: '后端允许 1–4；留空保留。' })}<div class="field"><span class="field-label">获胜策略</span><p class="hint">first_valid_success<br>采用首个有效成功响应</p></div></div></section><section class="form-section"><details class="advanced"><summary>高级超时策略 JSON</summary>${field('timeout_policy', '超时策略', resolvedSetting(data, 'timeout_policy') === undefined ? '' : JSON.stringify(resolvedSetting(data, 'timeout_policy'), null, 2), { type: 'textarea', help: '留空保留当前策略。后端将验证 default / rules 结构。' })}${jsonDetails(data.resolved_fields, '查看字段来源与当前解析值')}</details><div id="settings-error" class="notice danger" role="alert" hidden></div><div id="settings-validation" class="notice success" role="status" hidden></div></section></fieldset><div class="form-footer"><span class="hint settings-revision">修订 ${esc(String(data.revision).slice(0, 12))}</span><button class="button" type="button" id="validate-settings"${readOnly ? ' disabled' : ''}>${icon('shield')}仅校验</button><button class="button primary" type="submit"${readOnly ? ' disabled' : ''}>${icon('check')}保存设置</button></div></form>`;
}
function settingsMutation(form) {
  const data = state.settings; const set = {};
  for (const key of Object.keys(settingsLabels)) { const value = form.elements[key].value; if (value !== '') { const number = Number(value); if (!Number.isFinite(number) || number < 0) throw new Error(`${settingsLabels[key][0]} 必须是非负数字。`); if (number !== resolvedSetting(data, key)) set[`/preferences/${key}`] = number; } }
  const currentHedging = resolvedSetting(data, 'hedging') || {};
  const attempts = form.elements.max_inflight_attempts.value;
  if (attempts !== '' && (!Number.isInteger(Number(attempts)) || Number(attempts) < 1 || Number(attempts) > 4)) throw new Error('最大并发尝试数必须为 1–4 之间的整数。');
  if (currentHedging.enabled !== form.elements.hedging_enabled.checked || (attempts !== '' && Number(attempts) !== currentHedging.max_inflight_attempts)) set['/preferences/hedging'] = { ...currentHedging, enabled: form.elements.hedging_enabled.checked, ...(attempts !== '' ? { max_inflight_attempts: Number(attempts) } : {}), winner_policy: 'first_valid_success' };
  if (form.elements.timeout_policy.value.trim()) { const policy = parseJSON(form.elements.timeout_policy.value, '超时策略'); if (JSON.stringify(policy) !== JSON.stringify(resolvedSetting(data, 'timeout_policy'))) set['/preferences/timeout_policy'] = policy; }
  if (!Object.keys(set).length) throw new Error('没有检测到需要提交的更改。');
  return { revision: data.revision, operation_id: operationId(), changes: [{ provider: data.provider || '__uni_console_global_preferences__', set, remove: [], reset: false }] };
}
function operationId() { if (crypto.randomUUID) return crypto.randomUUID(); const bytes = new Uint8Array(16); crypto.getRandomValues(bytes); return `ui-${Array.from(bytes, b => b.toString(16).padStart(2, '0')).join('')}`; }
async function saveSettings(form, validateOnly) {
  if (!form.reportValidity() || form.dataset.busy) return;
  const errorBox = form.querySelector('#settings-error'), successBox = form.querySelector('#settings-validation'); errorBox.hidden = true; successBox.hidden = true;
  form.dataset.busy = 'true'; form.setAttribute('aria-busy', 'true'); form.querySelectorAll('button').forEach(b => { b.disabled = true; });
  try { const payload = settingsMutation(form); if (validateOnly) { await api.post('/settings/validate', payload); successBox.textContent = '后端校验通过。尚未保存任何更改。'; successBox.hidden = false; } else { await api.patch('/settings', payload); toast('设置已保存'); await render(); } }
  catch (error) { errorBox.textContent = error.message; errorBox.hidden = false; errorBox.tabIndex = -1; errorBox.focus(); }
  finally { delete form.dataset.busy; form.removeAttribute('aria-busy'); form.querySelectorAll('button').forEach(b => { b.disabled = false; }); }
}

const standalonePages = { logs: mountRequestLogs, 'log-settings': mountLogSettings, keys: mountKeys, workspace: mountWorkspace, testing: mountTesting, playground: mountPlayground, 'backend-logs': mountBackendLogs };
const pages = { overview, channels, plugins, interceptors, usage, settings };
async function render() {
  if (currentRoute === 'workspace' && !confirmWorkspaceLeave()) { window.history.replaceState(null, '', '#workspace'); return; }
  const token = ++generation; controller?.abort(); controller = new AbortController();
  const nextRoute = window.location.hash.slice(1).split('?')[0] || 'overview';
  if (!routes[nextRoute]) { window.location.hash = 'overview'; return; }
  const sameRoute = nextRoute === currentRoute && Boolean(main.innerHTML) && !main.innerHTML.includes('class="spinner"');
  currentRoute = nextRoute;
  const navRoute = currentRoute === 'log-settings' ? 'settings' : currentRoute === 'usage' ? 'logs' : currentRoute;
  document.querySelectorAll('[data-route]').forEach(link => { if (link.dataset.route === navRoute) link.setAttribute('aria-current', 'page'); else link.removeAttribute('aria-current'); });
  document.querySelector('#breadcrumb-current').textContent = routes[currentRoute][0]; document.title = `${routes[currentRoute][0]} · Miora 管理控制台`;
  if (!sameRoute) main.innerHTML = head() + (currentRoute === 'settings' ? settingsNav('settings') : '') + loading();
  main.setAttribute('aria-busy', 'true'); closeMenu();
  try {
    if (standalonePages[currentRoute]) {
      if (!sameRoute) main.innerHTML = '';
      await standalonePages[currentRoute](main, { signal: controller.signal });
    } else {
      const html = await pages[currentRoute](); if (token !== generation) return; main.innerHTML = html; bindPage();
    }
  }
  catch (error) { if (token !== generation) return; main.innerHTML = head() + (currentRoute === 'settings' ? settingsNav('settings') : '') + `<section class="card">${failure(error)}</section>`; }
  finally { if (token === generation) main.removeAttribute('aria-busy'); }
}
function bindPage() {
  const channelSearch = main.querySelector('#channel-search');
  if (channelSearch) {
    const channelType = main.querySelector('#f-channel_type_filter');
    const clearFilters = main.querySelector('#clear-channel-filters');
    const channelGroup = main.querySelector('#f-channel_group_filter');
    const filterChannels = () => {
      const query = channelSearch.value.trim().toLowerCase(), engine = channelType.value, group = channelGroup.value;
      let count = 0;
      main.querySelectorAll('tr[data-search]').forEach((row, index) => {
        row.hidden = !row.dataset.search.includes(query) || Boolean(engine && row.dataset.engine !== engine) || Boolean(group && !(state.channels.channels[index].groups || ['default']).includes(group));
        if (!row.hidden) count++;
      });
      main.querySelectorAll?.('[data-channel-group-pill]')?.forEach?.(btn => btn.setAttribute?.('aria-pressed', String(btn.dataset.channelGroupPill === group)));
      const total = state.channels.channels.length;
      main.querySelector('#search-empty').hidden = count !== 0 || !total;
      main.querySelector('#channel-filter-count').textContent = count === total ? `共 ${fmt(total)} 个渠道` : `显示 ${fmt(count)} / ${fmt(total)} 个渠道`;
      clearFilters.disabled = !channelSearch.value && !engine && !group;
      const params = new URLSearchParams();
      if (channelSearch.value) params.set('q', channelSearch.value);
      if (engine) params.set('type', engine);
      if (group) params.set('group', group);
      const route = `#channels${params.size ? `?${params}` : ''}`;
      if (window.location.hash !== route) window.history.replaceState(null, '', route);
    };
    channelSearch.addEventListener('input', filterChannels);
    channelType.addEventListener('change', filterChannels);
    channelGroup.addEventListener('change', filterChannels);
    clearFilters.addEventListener('click', () => { channelSearch.value = ''; channelType.value = ''; channelGroup.value = ''; filterChannels(); channelSearch.focus(); });
    filterChannels();
  }
  const usageForm = document.querySelector('#usage-filter');
  usageForm?.addEventListener('submit', event => { event.preventDefault(); const form = event.target; usageDays = Number(form.elements.days.value); usageModel = form.elements.model.value.trim(); usageChannel = form.elements.channel.value.trim(); usageGroup = form.elements.group.value; render(); });
  usageForm?.elements.group.addEventListener('change', event => {
    usageGroup = event.target.value;
    if (state.usage?.available) document.querySelector('#usage-breakdown').innerHTML = usageBreakdown(state.usage, usageFilters());
  });
  document.querySelector('#f-trend_metric')?.addEventListener('change', event => {
    document.querySelector('#usage-trend-content').innerHTML = usageTrend(state.usage, usageFilters(), event.target.value);
  });
  document.querySelector('#settings-form')?.addEventListener('submit', event => { event.preventDefault(); saveSettings(event.target, false); });
  document.querySelector('#validate-settings')?.addEventListener('click', () => saveSettings(document.querySelector('#settings-form'), true));
}
main.addEventListener('click', async event => {
  const copyChip = event.target.closest?.('[data-copy-text]');
  if (copyChip?.dataset?.copyText) {
    const text = copyChip.dataset.copyText;
    try { await navigator.clipboard.writeText(text); toast(`已复制：${text}`); } catch {}
    return;
  }
  const target = event.target.closest('[data-action]'); if (!target || target.disabled) return;
  if (target.dataset.action === 'expand-tags') {
    const container = target.closest('.tags');
    container?.querySelectorAll('[data-extra-tag]').forEach(el => { el.hidden = false; });
    target.hidden = true;
    return;
  }
  if (target.dataset.action === 'filter-channel-group') {
    const selectEl = main.querySelector('#f-channel_group_filter');
    if (selectEl) { selectEl.value = target.dataset.channelGroupPill || ''; selectEl.dispatchEvent(new Event('change')); }
    return;
  }
  const index = target.dataset.index === undefined ? undefined : Number(target.dataset.index);
  const actions = { 'check-update': openUpdateDialog, 'channel-balance': () => showChannelBalance(index), 'quick-balance': () => quickRefreshBalance(index), 'refresh-balances': batchRefreshBalances, 'channel-accounts': () => openChannelAccounts(state.channels.channels[index].provider), 'test-channel': () => openChannelTestDialog(state.channels.channels[index], { revision: state.channels?.revision, writable: state.channels?.writable, onUpdated: render }), 'batch-test-channels': batchTestChannels, 'toggle-channel': () => mutateChannels([state.channels.channels[index].provider], { enabled: state.channels.channels[index].enabled === false }), 'select-channels': () => { const boxes = [...main.querySelectorAll('[data-channel-select]')].filter(box => !box.closest('tr').hidden); const allChecked = boxes.length > 0 && boxes.every(box => box.checked); for (const box of boxes) box.checked = !allChecked; }, 'enable-channels': () => mutateChannels(selectedChannels(), { enabled: true }), 'disable-channels': () => mutateChannels(selectedChannels(), { enabled: false }), 'group-channels': () => groupChannels(), 'manage-groups': () => openGroupManager({ onChanged: render }), refresh: () => authenticated ? render() : window.location.reload(), 'create-channel': () => editChannel(), 'create-channel-from-plugin': async () => { const plugin = state.plugins.plugins[index]; const initialType = plugin.kind !== 'builtin' ? `service:${plugin.id}` : `builtin:${plugin.manifest?.engine || plugin.id.replace(/^builtin-/, '')}`; window.location.hash = 'channels'; await render(); await editChannel(undefined, initialType); }, 'edit-channel': () => editChannel(index), 'delete-channel': () => deleteChannel(index), 'copy-channel': () => copyChannel(index), 'channel-usage': () => { usageChannel = state.channels.channels[index].provider; usageModel = ''; usageGroup = 'model'; window.location.hash = 'overview'; }, 'create-plugin': () => openPluginUpload(state.plugins.plugins, render), 'upload-plugin-update': () => openPluginUpload(state.plugins.plugins, render, state.plugins.plugins[index]), 'plugin-group': () => { pluginGroup = target.dataset.group === 'builtin' ? 'builtin' : 'installed'; return render(); }, 'usage-detail': () => openUsageDetail((usageGroup === 'channel' ? state.usage.channels : state.usage.usage)[index]), 'plugin-logs': () => openPluginLogs(state.plugins.plugins[index]), 'plugin-detail': () => pluginDetail(index), 'toggle-plugin': () => togglePlugin(index), 'create-interceptor': () => editInterceptor(), 'edit-interceptor': () => editInterceptor(index), 'toggle-interceptor': () => mutateInterceptor(index, 'toggle'), 'delete-interceptor': () => mutateInterceptor(index, 'delete') };
  if (!actions[target.dataset.action]) return;
  target.disabled = true;
  try { await actions[target.dataset.action](); }
  catch (error) { dialog({ title: '操作暂不可用', body: notice(error.message, 'danger'), submitLabel: '关闭', onSubmit: async () => {} }); }
  finally { target.disabled = false; }
});
const sidebar = document.querySelector('#sidebar'), menuToggle = document.querySelector('#menu-toggle'), backdrop = document.querySelector('#sidebar-backdrop');
const narrowViewport = window.matchMedia('(max-width: 760px)');
function closeMenu() { sidebar.classList.remove('is-open'); sidebar.inert = narrowViewport.matches; backdrop.hidden = true; menuToggle.setAttribute('aria-expanded', 'false'); }
narrowViewport.addEventListener('change', closeMenu);
closeMenu();
menuToggle.addEventListener('click', () => { const open = !sidebar.classList.contains('is-open'); sidebar.classList.toggle('is-open', open); sidebar.inert = !open && narrowViewport.matches; backdrop.hidden = !open; menuToggle.setAttribute('aria-expanded', String(open)); if (open) sidebar.querySelector('a').focus(); });
backdrop.addEventListener('click', closeMenu);
document.addEventListener('keydown', event => {
  if (!sidebar.classList.contains('is-open') || document.querySelector('#editor-dialog').open) return;
  if (event.key === 'Escape') { closeMenu(); menuToggle.focus(); }
  if (event.key === 'Tab' && narrowViewport.matches) {
    const controls = [...sidebar.querySelectorAll('a[href], button:not(:disabled)')]; const first = controls[0], last = controls.at(-1);
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  }
});
document.querySelector('#check-update')?.addEventListener('click', openUpdateDialog);
document.querySelector('#logout').addEventListener('click', async () => { await api.post('/logout'); api.setSession(null); window.location.replace('/login'); });
window.addEventListener('hashchange', () => { if (authenticated) render(); });
mountIcons();
try { const session = await api.session(); authenticated = true; document.querySelector('#account-name').textContent = session.user?.username || '管理员'; document.querySelector('#session-state').textContent = '管理会话已验证'; await render(); }
catch (error) { document.querySelector('#session-state').textContent = '会话验证失败'; main.innerHTML = `<section class="card">${failure(error)}<div class="form-footer"><a class="button primary" href="/login">前往登录${icon('arrow')}</a></div></section>`; }
