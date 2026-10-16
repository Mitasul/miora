import * as api from './api.js';
import { icon, aiIcon, modelBrand } from './icons.js';
import { esc, fmt, field, select, notice, empty, loading, dialog } from './ui.js';
import { CAPTURE_FIELDS, logsNav } from './log-settings.js';

const fieldNames = new Map(CAPTURE_FIELDS.map(item => [item.key, item.label]));
export function formatBytes(value) {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) return '—';
  if (value < 1024) return `${value} B`;
  if (value < 1048576) return `${Number((value / 1024).toFixed(2))} KiB`;
  return `${Number((value / 1048576).toFixed(2))} MiB`;
}
export function formatLogTime(value) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return '—';
  const date = new Date(value); return Number.isNaN(date.getTime()) ? '—' : date.toLocaleString('zh-CN', { hour12: false });
}
export function formatDuration(value) { return typeof value === 'number' && Number.isFinite(value) ? `${Number(value.toFixed(2))} ms` : '—'; }
export function parseLogFilters(values) {
  const filters = {};
  for (const key of ['request_id', 'channel', 'model']) if (String(values[key] ?? '').trim()) filters[key] = String(values[key]).trim();
  if (String(values.status ?? '').trim()) {
    const status = Number(values.status);
    if (!Number.isInteger(status) || status < 100 || status > 599) throw new Error('HTTP 状态筛选必须是 100–599 的精确状态码。');
    filters.status = status;
  }
  for (const [input, output] of [['from', 'from_ms'], ['to', 'to_ms']]) {
    if (!values[input] && values[output] === undefined) continue;
    const time = values[output] === undefined ? new Date(values[input]).getTime() : Number(values[output]);
    if (!Number.isSafeInteger(time) || time < 0) throw new Error('请输入有效且不早于 Unix 纪元的日期与时间。');
    filters[output] = time;
  }
  if (filters.from_ms !== undefined && filters.to_ms !== undefined && filters.from_ms >= filters.to_ms) throw new Error('开始时间必须早于结束时间；结束边界不包含在筛选内。');
  return filters;
}
export function logFiltersFromHash(hash) {
  const params = new URLSearchParams(hash.split('?')[1] || '');
  return parseLogFilters(Object.fromEntries(params));
}
const localDateTime = milliseconds => {
  const date = new Date(milliseconds);
  return new Date(milliseconds - date.getTimezoneOffset() * 60_000).toISOString().slice(0, -1);
};
export function logQuery(filters, page = 1, pageSize = 50) {
  const query = new URLSearchParams({ page: String(page), page_size: String(pageSize) });
  for (const key of ['request_id', 'channel', 'model', 'status', 'from_ms', 'to_ms']) if (filters[key] !== undefined && filters[key] !== null && filters[key] !== '') query.set(key, String(filters[key]));
  return query.toString();
}
export function isFieldAvailable(meta) { return meta?.available === true && meta.availability === 'available'; }
export function logContentPath(id, fieldName, attempt, download = false) {
  if (!fieldNames.has(fieldName)) throw new Error('未知日志内容字段。');
  const query = new URLSearchParams({ field: fieldName });
  if (attempt !== null && attempt !== undefined) {
    if (!Number.isSafeInteger(attempt) || attempt < 0) throw new Error('无效的尝试序号。');
    query.set('attempt', String(attempt));
  }
  if (!download) query.set('limit', '65536');
  return `/request-logs/${encodeURIComponent(String(id))}/${download ? 'download' : 'content'}?${query}`;
}
export function cleanupRequest(action, fields, filters) {
  if (!['clear_fields', 'delete_rows'].includes(action)) throw new Error('请选择有效的清理动作。');
  if (action === 'clear_fields' && (!fields.length || fields.some(name => !fieldNames.has(name)))) throw new Error('请选择至少一个有效的原始内容字段。');
  return { dry_run: true, action, fields: action === 'clear_fields' ? [...new Set(fields)] : [], filters: { ...filters } };
}
export function fieldCompletenessLabel(value) {
  return value === true ? '捕获时完整' : value === false ? '未完整捕获 / 保留' : '未提供';
}
export function completionLabel(value, relay = false) {
  if (value === true) return relay ? '转发至 HTTP 栈完成' : '已观察到完整上游结束';
  if (value === false) return relay ? '转发尚未完整结束' : '未观察到完整上游结束';
  return '未观察';
}
const stateLabels = { available: '已捕获', disabled: '未启用捕获', retention_zero: '保留时间为 0', unsupported: '不支持捕获', truncated: '捕获已截断', redaction_failed: '脱敏失败，未保留', size_limit: '超过大小限制', memory_limit: '超过内存限制', not_observed: '未观察', expired: '已过期', cleaned: '已清理', cleared: '已清理', evicted: '已按预算淘汰', missing: '内容不存在' };
const stateLabel = value => stateLabels[value] || value || '未提供';
const statusBadge = status => typeof status === 'number' ? `<span class="badge ${status >= 400 ? 'red' : status >= 200 && status < 300 ? 'green' : ''}">${status}</span>` : '<span class="muted">—</span>';
const definition = rows => `<dl class="definition-list">${rows.map(([label, value]) => `<div><dt>${esc(label)}</dt><dd>${esc(value !== null && typeof value === 'object' ? JSON.stringify(value) : value ?? '—')}</dd></div>`).join('')}</dl>`;
const filterSummary = filters => {
  const values = [];
  if (filters?.from_ms !== undefined) values.push(['开始时间（含）', formatLogTime(filters.from_ms)]);
  if (filters?.to_ms !== undefined) values.push(['结束时间（不含）', formatLogTime(filters.to_ms)]);
  for (const [key, label] of [['request_id', 'Request ID'], ['channel', '渠道'], ['model', '模型'], ['status', 'HTTP 状态']]) if (filters?.[key] !== undefined && filters[key] !== '') values.push([label, filters[key]]);
  if (Array.isArray(filters?.ids)) values.push(['指定日志 ID 数量', filters.ids.length]);
  return values.length ? definition(values) : notice('筛选范围为全部符合条件的新请求日志，不仅限当前分页。', 'warning');
};
const safeJSON = value => `<pre>${esc(JSON.stringify(value, null, 2))}</pre>`;
const metrics = entries => `<div class="log-result-metrics">${entries.map(([label, value]) => `<div><span>${esc(label)}</span><strong>${esc(value)}</strong></div>`).join('')}</div>`;
const usageText = usage => {
  if (!usage || typeof usage !== 'object') return '—';
  if (typeof usage.total_tokens === 'number') return `${fmt(usage.total_tokens)} Tokens`;
  const observed = Object.entries(usage).filter(([, value]) => typeof value === 'number');
  const labels = { input_tokens: '输入', output_tokens: '输出', total_tokens: '总计', cache_read_tokens: '缓存读取', cache_write_tokens: '缓存写入' };
  return observed.length ? observed.map(([name, value]) => `${labels[name] || name}: ${fmt(value)}`).join(' / ') : '—';
};
const filterFields = (values, prefix = '') => `<div class="request-log-filters">${field(`${prefix}from`, '开始时间（包含）', values.from || '', { type: 'datetime-local', step: '0.001' })}${field(`${prefix}to`, '结束时间（不包含）', values.to || '', { type: 'datetime-local', step: '0.001' })}${field(`${prefix}request_id`, 'Request ID', values.request_id || '', { placeholder: '精确匹配' })}${field(`${prefix}channel`, '渠道', values.channel || '', { placeholder: '精确渠道名称' })}${field(`${prefix}model`, '模型', values.model || '', { placeholder: '匹配请求 / 发送 / 响应模型' })}${field(`${prefix}status`, 'HTTP 状态', values.status ?? '', { type: 'number', min: 100, step: 1, placeholder: '例如 200，精确匹配' })}</div>`;

export async function mountRequestLogs(root, { signal } = {}) {
  let page = 1, pageSize = 50, filters = logFiltersFromHash(window.location.hash), filterDraft = { ...filters, from: filters.from_ms === undefined ? '' : localDateTime(filters.from_ms), to: filters.to_ms === undefined ? '' : localDateTime(filters.to_ms) }, rows = [], listSequence = 0, listController;
  let closeDetail = null;
  const live = () => !signal?.aborted;
  root.innerHTML = `<div class="page-heading"><h1>请求日志</h1><div class="page-actions"><button class="button" type="button" data-request-log-action="refresh">${icon('refresh')}刷新</button><button class="button" type="button" data-request-log-action="cleanup">${icon('trash')}手工清理</button></div></div>${logsNav('logs')}<form id="request-log-filter" class="card"><div class="card-body">${filterFields(filterDraft)}<div class="filter-form-actions"><span class="hint">时间使用浏览器本地时区；筛选均为精确匹配。</span><button class="button" type="button" data-request-log-action="reset">重置</button><button class="button primary" type="submit">${icon('filter')}筛选</button></div><p class="notice danger request-log-filter-error" role="alert" hidden></p></div></form><div id="request-log-cleanup-result" hidden></div><section class="card request-log-list-card" aria-label="请求日志列表"><div id="request-log-list">${loading()}</div></section>`;
  const form = root.querySelector('#request-log-filter');
  function draftValues(target = form, prefix = '') { return Object.fromEntries(['from', 'to', 'request_id', 'channel', 'model', 'status'].map(name => [name, target.elements[`${prefix}${name}`].value])); }
  async function loadPage(reset = false) {
    if (!live()) return;
    if (reset) page = 1;
    const sequence = ++listSequence; listController?.abort(); listController = new AbortController();
    const box = root.querySelector('#request-log-list'); box.innerHTML = loading(); box.setAttribute('aria-busy', 'true');
    try {
      const data = await api.get(`/request-logs?${logQuery(filters, page, pageSize)}`, { signal: listController.signal });
      if (!live() || sequence !== listSequence) return;
      rows = data.rows; page = data.page; pageSize = data.page_size;
      const totalPages = Math.max(1, Math.ceil(data.total / pageSize));
      box.innerHTML = rows.length ? `<div class="table-scroll" tabindex="0" role="region" aria-label="请求摘要表，可横向滚动"><table class="request-log-table"><thead><tr><th>时间 / Request ID</th><th>渠道 / 请求模型</th><th>状态</th><th>耗时 / 首字节</th><th>首文本</th><th>用量</th><th>重试 / 尝试</th><th>操作</th></tr></thead><tbody>${rows.map((row, index) => `<tr><td><span class="log-time">${esc(formatLogTime(row.at_ms))}</span><code class="log-id" title="${esc(row.request_id)}">${esc(row.request_id || '—')}</code></td><td><strong>${esc(row.channel ?? '—')}</strong><span class="subline log-model-name" title="${esc(row.requested_model)}">${aiIcon(modelBrand(row.requested_model ?? ''))}${esc(row.requested_model ?? '—')}</span></td><td>${statusBadge(row.status)}${row.active ? '<span class="badge orange">进行中</span>' : ''}${row.error_stage ? `<span class="subline">${esc(row.error_stage)}</span>` : ''}</td><td>${esc(formatDuration(row.duration_ms))}<span class="subline">首字节 ${esc(formatDuration(row.first_byte_ms))}</span></td><td>${esc(formatDuration(row.first_text_ms))}</td><td class="log-usage">${esc(usageText(row.usage))}</td><td>—<span class="subline">${Array.isArray(row.attempts) ? `${fmt(row.attempts.length)} 次尝试` : '尝试数未提供'}</span></td><td><button class="button small" type="button" data-request-log-action="detail" data-row-index="${index}">${icon('eye')}详情</button></td></tr>`).join('')}</tbody></table></div>` : empty('没有匹配的请求日志', '调整筛选，或检查“日志与隐私”中的启用和保留配置。只产生真实请求后才有日志。', 'file');
      box.insertAdjacentHTML('beforeend', `<div class="log-pagination"><span>共 ${fmt(data.total)} 条 · 第 ${fmt(page)} / ${fmt(totalPages)} 页</span><div><button class="button small" type="button" data-request-log-action="previous" ${page <= 1 ? 'disabled' : ''}>上一页</button><button class="button small" type="button" data-request-log-action="next" ${page >= totalPages ? 'disabled' : ''}>下一页</button></div></div>`);
    } catch (error) {
      if (live() && sequence === listSequence) box.innerHTML = `${notice(error.message, 'danger')}<div class="card-body"><button class="button" type="button" data-request-log-action="refresh">${icon('refresh')}重试</button></div>`;
    } finally { if (live() && sequence === listSequence) box.removeAttribute('aria-busy'); }
  }
  function showCleanupResult(result) {
    const skipped = result.skipped_rows;
    const box = root.querySelector('#request-log-cleanup-result');
    box.innerHTML = `<section class="card cleanup-result"><div class="card-header"><div><h2>清理执行结果</h2><p>${result.action === 'delete_rows' ? '仅删除新的请求日志行；原有 stats / 用量摘要不受影响。' : '仅清理所选原始字段，保留请求日志摘要与统计。'}</p></div></div><div class="card-body">${metrics([['实际影响日志行', fmt(result.affected_rows)], ['实际清理字段', fmt(result.cleared_fields)], ['实际释放内容', formatBytes(result.freed_bytes)], ['跳过日志行', fmt(skipped)]])}${skipped > 0 ? notice('部分目标未被处理。下方展示实际返回的跳过计数和相关信息，不将部分完成伪装为全部成功。', 'warning') : ''}<details class="advanced"><summary>查看实际执行结果</summary>${safeJSON(result)}</details></div></section>`; box.hidden = false; box.scrollIntoView({ block: 'nearest' });
  }
  function beginCleanup() {
    dialog({ title: '手工清理请求日志', submitLabel: '预览清理影响', body: notice('第一步只预览目标与预计空间；第二步必须确认后才执行。进行中的请求会被排除。') + select('cleanup_action', '清理动作', [{ value: 'clear_fields', label: '清理原始字段 · 保留日志摘要' }, { value: 'delete_rows', label: '删除请求日志行 · 不删原 stats 摘要' }], 'clear_fields') + `<div id="cleanup-fields"><span class="field-label">要清理的原始字段</span><div class="capture-field-grid">${CAPTURE_FIELDS.map(item => `<label class="capture-option"><input type="checkbox" name="cleanup_fields" value="${item.key}"><span>${item.label}</span></label>`).join('')}</div></div><h3 class="cleanup-filter-title">目标筛选</h3>${filterFields(filterDraft, 'cleanup_')}<p class="hint">空筛选代表全部符合条件的新请求日志，不仅是当前分页。</p>`, onMount: cleanupForm => {
      cleanupForm.elements.cleanup_action.addEventListener('change', () => { cleanupForm.querySelector('#cleanup-fields').hidden = cleanupForm.elements.cleanup_action.value !== 'clear_fields'; });
    }, onSubmit: async cleanupForm => {
      const body = cleanupRequest(cleanupForm.elements.cleanup_action.value, [...cleanupForm.querySelectorAll('[name="cleanup_fields"]:checked')].map(input => input.value), parseLogFilters(draftValues(cleanupForm, 'cleanup_')));
      const result = await api.request('/request-logs/cleanup', { method: 'POST', body, signal });
      if (!live()) return;
      setTimeout(() => { if (live()) confirmCleanup(result); }, 0);
    } });
  }
  function confirmCleanup(preview) {
    const hasTargets = preview.matched_rows > 0;
    dialog({ title: '确认清理范围', submitLabel: hasTargets ? '确认执行清理' : '关闭', danger: hasTargets, showCancel: hasTargets, body: notice(preview.action === 'delete_rows' ? '将删除预览中冻结的请求日志行及其原始内容。原有 stats / 用量摘要不会删除；执行期间新增的请求不在本次目标内。' : '将清理预览中冻结的原始字段，保留日志行摘要和统计。已清理的内容不可恢复或下载。', 'warning') + metrics([['匹配日志行', fmt(preview.matched_rows)], ['排除的活跃行', fmt(preview.active_excluded)], ['预计原始内容', formatBytes(preview.estimated_bytes)]]) + definition([['动作', preview.action === 'delete_rows' ? '删除请求日志行' : '清理原始字段'], ['字段', preview.fields.map(name => fieldNames.get(name) || name).join('、') || '全部行内容随日志行删除'], ['确认有效期至', formatLogTime(preview.expires_at_ms)]]) + `${filterSummary(preview.filters)}<details class="advanced"><summary>冻结的筛选条件（原始值）</summary>${safeJSON(preview.filters)}</details>${!hasTargets ? notice('没有可执行的匹配目标，本次不会发出清理确认。') : ''}`, onSubmit: async () => {
      if (!hasTargets) return;
      const result = await api.request('/request-logs/cleanup', { method: 'POST', body: { dry_run: false, confirmation_token: preview.confirmation_token }, signal });
      if (!live()) return;
      showCleanupResult(result); await loadPage(true);
    } });
  }
  async function openDetail(id) {
    closeDetail?.();
    const detailController = new AbortController(); let closed = false, data, activeTab = 'overview', selectedAttempt = null, renderSequence = 0;
    const contentControllers = new Set();
    closeDetail = () => { closed = true; detailController.abort(); contentControllers.forEach(controller => controller.abort()); contentControllers.clear(); };
    const alive = () => live() && !closed;
    const tabs = [['overview', '概览'], ['request', '客户端请求'], ['upstream-request', '上游请求'], ['upstream-response', '上游响应'], ['response', '返回客户端'], ['attempts', '尝试路径']];
    let detailForm;
    function fieldsForTab() {
      const names = CAPTURE_FIELDS.filter(item => item.stage === activeTab).map(item => item.key);
      return names.map(name => data.fields.find(meta => meta.field === name && (activeTab.startsWith('upstream-') ? (meta.attempt ?? null) === selectedAttempt : (meta.attempt ?? null) === null)) || { field: name, attempt: selectedAttempt, available: false, availability: 'missing', capture_state: 'not_observed', reason: '后端未返回此阶段字段元数据' });
    }
    function renderDetail() {
      if (!alive()) return;
      const sequence = ++renderSequence, record = data.record;
      const tabsNode = detailForm.querySelector('.request-log-tabs');
      tabsNode.innerHTML = tabs.map(([key, label]) => `<button type="button" role="tab" id="request-log-tab-${key}" data-log-tab="${key}" aria-selected="${key === activeTab}" tabindex="${key === activeTab ? 0 : -1}" aria-controls="request-log-detail-panel">${label}</button>`).join('');
      const panel = detailForm.querySelector('#request-log-detail-panel');
      panel.setAttribute('aria-labelledby', `request-log-tab-${activeTab}`);
      if (activeTab === 'overview') {
        panel.innerHTML = `<div class="log-detail-heading"><div><h3>${esc(record.request_id || record.id)}</h3><p>${esc(formatLogTime(record.at_ms))} · ${esc(record.channel ?? '—')}</p></div>${statusBadge(record.status)}</div><div class="log-detail-grid">${definition([['请求方法', record.method], ['请求端点', record.endpoint], ['请求模型', record.requested_model], ['实际发送模型', record.sent_model], ['观察到的响应模型', record.response_model], ['错误阶段', record.error_stage], ['总耗时', formatDuration(record.duration_ms)], ['首字节', formatDuration(record.first_byte_ms)], ['首文本', formatDuration(record.first_text_ms)]])}${definition([['上游完整性', completionLabel(record.upstream_complete)], ['下游转发完整性', completionLabel(record.relay_complete, true)], ['请求活跃', (data.active ?? record.active) === true ? '进行中' : (data.active ?? record.active) === false ? '已结束' : '未提供'], ['取消', record.cancelled === true ? '是' : record.cancelled === false ? '否' : '未观察'], ['流式', record.stream === true ? '是' : record.stream === false ? '否' : '未观察'], ['不完整标记', record.incomplete === true ? '是' : record.incomplete === false ? '否' : '未观察'], ['用量', usageText(record.usage)]])}</div>${notice('上游读到结束与向 HTTP 栈转发结束是两种观测。两者都不等于客户端已经接收或处理全部内容。未观察到的响应模型显示为 —。')}`;
      } else if (activeTab === 'attempts') {
        const attempts = record.attempts;
        panel.innerHTML = `<h3>实际尝试路径</h3><p class="hint">每次尝试单独记录；不把尝试数减一当成重试数，避免混淆并行尝试。</p>${attempts.length ? `<div class="log-attempt-list">${attempts.map(attempt => `<article class="log-attempt"><h4>尝试 ${esc(attempt.index ?? '—')}</h4>${Object.entries(attempt).filter(([, value]) => value === null || typeof value !== 'object').map(([name, value]) => `<div><span>${esc(name)}</span><code>${esc(value !== null && typeof value === 'object' ? JSON.stringify(value) : value ?? '—')}</code></div>`).join('')}<details class="advanced"><summary>此尝试的实际元数据</summary>${safeJSON(attempt)}</details></article>`).join('')}</div>` : empty('没有尝试元数据', '后端未观察或未记录尝试路径，不补造记录。', 'channels')}`;
      } else {
        const upstream = activeTab.startsWith('upstream-');
        const attempts = [...new Set(data.fields.filter(meta => meta.field.startsWith('upstream_')).map(meta => meta.attempt ?? null))];
        const selector = upstream && attempts.length ? `<div class="log-attempt-picker"><label for="log-attempt-select">查看尝试</label><select id="log-attempt-select">${attempts.map(attempt => `<option value="${attempt === null ? 'root' : attempt}" ${attempt === selectedAttempt ? 'selected' : ''}>${attempt === null ? '请求级上游观测' : `尝试 ${attempt}`}</option>`).join('')}</select></div>` : '';
        panel.innerHTML = selector + fieldsForTab().map((meta, index) => `<section class="log-field-card"><div class="log-field-title"><h3>${esc(fieldNames.get(meta.field) || meta.field)}</h3><span class="badge ${isFieldAvailable(meta) ? 'purple' : 'orange'}">${esc(stateLabel(meta.capture_state))}</span></div>${definition([['观察到的原始字节', formatBytes(meta.observed_bytes)], ['实际已保存字节', formatBytes(meta.saved_bytes)], ['字段捕获完整性', fieldCompletenessLabel(meta.complete)], ['捕获状态', meta.capture_state], ['存储可用性', meta.availability], ['可用性原因', meta.availability_reason ?? '未提供额外原因'], ['内容类型', meta.content_type], ['内容表示', meta.representation ?? '未提供'], ['脱敏标记', meta.redacted === true ? '服务端已脱敏' : meta.redacted === false ? '未标记脱敏' : '未提供'], ['捕获原因', meta.reason ?? '未提供额外原因']])}${isFieldAvailable(meta) ? `<div class="log-content-actions"><button class="button" type="button" data-log-content="${index}">${icon('eye')}查看已保存内容</button><button class="button" type="button" data-log-download="${index}">${icon('file')}下载已保存文件</button></div>` : `<p class="log-content-unavailable">${esc(stateLabel(meta.availability))}：当前没有可查看或下载的存储内容。${esc(meta.availability_reason || meta.reason || '')}</p>`}<div class="log-field-content" data-content-slot="${index}" hidden></div></section>`).join('');
        panel.querySelector('#log-attempt-select')?.addEventListener('change', event => { selectedAttempt = event.target.value === 'root' ? null : Number(event.target.value); renderDetail(); });
        panel.querySelectorAll('[data-log-content]').forEach(button => button.addEventListener('click', () => showContent(fieldsForTab()[Number(button.dataset.logContent)], Number(button.dataset.logContent), sequence)));
        panel.querySelectorAll('[data-log-download]').forEach(button => button.addEventListener('click', () => downloadContent(fieldsForTab()[Number(button.dataset.logDownload)])));
      }
    }
    async function loadDetail() {
      const next = await api.get(`/request-logs/${encodeURIComponent(String(id))}`, { signal: detailController.signal });
      if (!alive()) return;
      data = next;
      const indices = data.fields.filter(meta => meta.field.startsWith('upstream_') && Number.isSafeInteger(meta.attempt)).map(meta => meta.attempt);
      if (selectedAttempt === null && indices.length) selectedAttempt = Math.max(...indices);
      contentControllers.forEach(controller => controller.abort()); contentControllers.clear(); renderDetail();
    }
    async function showContent(meta, index, sequence) {
      if (!isFieldAvailable(meta)) return;
      const slot = detailForm.querySelector(`[data-content-slot="${index}"]`); slot.hidden = false; slot.innerHTML = '<p class="hint" role="status">正在读取已保存内容…</p>';
      const contentController = new AbortController(); contentControllers.add(contentController);
      try {
        const result = await api.get(logContentPath(id, meta.field, meta.attempt), { signal: contentController.signal });
        if (!alive() || sequence !== renderSequence) return;
        slot.innerHTML = `${result.preview_truncated ? notice('预览长度受限，以下不是完整已保存内容。需要更多内容时，可下载服务端当前仍可用的存储文件。', 'warning') : ''}${result.capture_state === 'truncated' ? notice('捕获本身已截断：下载文件也只包含实际保存的部分，不是完整原始传输。', 'warning') : ''}<div class="log-text-toolbar"><span>${esc(result.content_type || meta.content_type || '未提供 Content-Type')} · ${esc(result.encoding || '编码未提供')}</span><button class="button small" type="button" data-json-view>格式化 JSON</button></div><pre class="log-content-text" tabindex="0" aria-label="安全文本内容"></pre><p class="log-json-status hint" role="status"></p>`;
        const pre = slot.querySelector('pre'); pre.textContent = result.content;
        const toggle = slot.querySelector('[data-json-view]'); let formatted = false;
        toggle.addEventListener('click', () => {
          const status = slot.querySelector('.log-json-status');
          if (formatted) { pre.textContent = result.content; toggle.textContent = '格式化 JSON'; status.textContent = ''; formatted = false; return; }
          try { pre.textContent = JSON.stringify(JSON.parse(result.content), null, 2); toggle.textContent = '显示原始文本'; formatted = true; status.textContent = '只格式化显示，不修改已保存文件。'; }
          catch { status.textContent = '内容不是完整有效的 JSON，继续以纯文本显示。HTML / SVG 不会执行。'; }
        });
      } catch (error) { if (alive() && sequence === renderSequence) { slot.textContent = error.message; slot.setAttribute('role', 'alert'); if ([404, 410].includes(error.status)) { try { await loadDetail(); } catch (refreshError) { const errorBox = detailForm.querySelector('.log-detail-error'); errorBox.textContent = refreshError.message; errorBox.hidden = false; } } } }
      finally { contentControllers.delete(contentController); }
    }
    function downloadContent(meta) {
      if (!isFieldAvailable(meta)) return;
      const link = document.createElement('a'); link.href = `/admin/api${logContentPath(id, meta.field, meta.attempt, true)}`; link.download = ''; link.rel = 'noreferrer'; document.body.append(link); link.click(); link.remove();
    }
    detailForm = dialog({ title: '请求日志详情', size: 'request-log', submitLabel: '关闭', showCancel: false, body: `<div class="request-log-tabs" role="tablist" aria-label="请求日志阶段"></div><div class="log-detail-tools"><span class="hint">按需读取原始内容；状态以最近一次刷新为准。</span><button class="button small" type="button" data-detail-refresh>${icon('refresh')}刷新详情</button></div><div class="notice danger log-detail-error" role="alert" hidden></div><div id="request-log-detail-panel" role="tabpanel">${loading()}</div>`, onMount: mounted => {
      detailForm = mounted;
      mounted.insertBefore(mounted.querySelector('.request-log-tabs'), mounted.querySelector('.dialog-body'));
      mounted.querySelector('.request-log-tabs').addEventListener('click', event => { const tab = event.target.closest('[data-log-tab]'); if (!tab) return; activeTab = tab.dataset.logTab; contentControllers.forEach(controller => controller.abort()); contentControllers.clear(); renderDetail(); mounted.querySelector(`[data-log-tab="${activeTab}"]`).focus(); mounted.querySelector('.dialog-body').scrollTop = 0; });
      mounted.querySelector('.request-log-tabs').addEventListener('keydown', event => {
        const buttons = [...mounted.querySelectorAll('[data-log-tab]')], index = buttons.indexOf(document.activeElement);
        if (index < 0 || !['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
        event.preventDefault(); const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (index + (event.key === 'ArrowRight' ? 1 : -1) + buttons.length) % buttons.length;
        activeTab = buttons[next].dataset.logTab; renderDetail(); mounted.querySelector(`[data-log-tab="${activeTab}"]`).focus();
      });
      mounted.querySelector('[data-detail-refresh]').addEventListener('click', () => loadDetail().catch(error => { if (alive()) { const box = mounted.querySelector('.log-detail-error'); box.textContent = error.message; box.hidden = false; } }));
    }, onSubmit: async () => {}, onClose: closeDetail });
    try { await loadDetail(); }
    catch (error) { if (alive()) { detailForm.querySelector('#request-log-detail-panel').innerHTML = notice(error.message, 'danger'); } }
  }
  form.addEventListener('submit', event => {
    event.preventDefault(); const error = form.querySelector('.request-log-filter-error'); error.hidden = true;
    try { const nextDraft = draftValues(); const nextFilters = parseLogFilters(nextDraft); filterDraft = nextDraft; filters = nextFilters; loadPage(true); }
    catch (reason) { error.textContent = reason.message; error.hidden = false; }
  });
  root.addEventListener('click', event => {
    if (!live()) return;
    const button = event.target.closest('[data-request-log-action]'); if (!button || button.disabled) return;
    const action = button.dataset.requestLogAction;
    if (action === 'refresh') loadPage();
    if (action === 'previous') { page--; loadPage(); }
    if (action === 'next') { page++; loadPage(); }
    if (action === 'reset') { form.reset(); filters = {}; filterDraft = {}; loadPage(true); }
    if (action === 'cleanup') beginCleanup();
    if (action === 'detail') openDetail(rows[Number(button.dataset.rowIndex)].id);
  }, { signal });
  signal?.addEventListener('abort', () => { listSequence++; listController?.abort(); closeDetail?.(); const modal = document.querySelector('#editor-dialog'); if (modal?.open) modal.close(); }, { once: true });
  await loadPage();
}
