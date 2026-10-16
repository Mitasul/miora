import * as api from './api.js';
import { icon } from './icons.js';
import { field, select, checkbox, notice, loading, failure } from './ui.js';

export const CAPTURE_FIELDS = [
  { key: 'request_headers', label: '客户端请求头', stage: 'request' },
  { key: 'request_body', label: '客户端请求体', stage: 'request' },
  { key: 'upstream_request_headers', label: '上游请求头', stage: 'upstream-request' },
  { key: 'upstream_request_body', label: '上游请求体', stage: 'upstream-request' },
  { key: 'upstream_response_headers', label: '上游响应头', stage: 'upstream-response' },
  { key: 'upstream_response_body', label: '上游响应体', stage: 'upstream-response' },
  { key: 'response_headers', label: '返回客户端的响应头', stage: 'response' },
  { key: 'response_body', label: '返回客户端的响应体', stage: 'response' },
];
export const settingsNav = active => `<nav class="section-nav" aria-label="系统设置分类"><a href="#settings" ${active === 'settings' ? 'aria-current="page"' : ''}>${icon('settings')}请求调度</a><a href="#log-settings" ${active === 'log-settings' ? 'aria-current="page"' : ''}>${icon('shield')}日志与隐私</a></nav>`;
export const logsNav = active => `<nav class="section-nav" aria-label="日志与统计分类"><a href="#logs" ${active === 'logs' ? 'aria-current="page"' : ''}>${icon('file')}请求日志</a><a href="#backend-logs" ${active === 'backend-logs' ? 'aria-current="page"' : ''}>${icon('code')}后台日志</a><a href="#usage" ${active === 'usage' ? 'aria-current="page"' : ''}>${icon('chart')}用量与管理审计</a><a href="#log-settings" ${active === 'log-settings' ? 'aria-current="page"' : ''}>${icon('settings')}日志设置</a></nav>`;

export function unitsToBytes(value, unit, label = '容量') {
  if (!['KiB', 'MiB'].includes(unit)) throw new Error('未知容量单位。');
  if (value === null || value === undefined || String(value).trim() === '') throw new Error(`请填写${label}，0 与未填写不同。`);
  const number = Number(value), bytes = number * (unit === 'KiB' ? 1024 : 1048576);
  if (!Number.isFinite(number) || number < 0 || !Number.isSafeInteger(bytes)) throw new Error(`${label}必须能准确换算为非负整数字节。`);
  return bytes;
}
export function bytesToUnits(value, unit) {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) return '';
  return value / (unit === 'KiB' ? 1024 : 1048576);
}
function integer(value, label, minimum = 0) {
  if (value === null || value === undefined || String(value).trim() === '') throw new Error(`请填写${label}。`);
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < minimum) throw new Error(`${label}必须是至少 ${minimum} 的整数。`);
  return number;
}
export function buildLogSettings(original, values, runtimeManaged) {
  const known = new Set(CAPTURE_FIELDS.map(field => field.key));
  const selected = values.capture_fields || [];
  if (!Array.isArray(selected) || selected.some(field => !known.has(field))) throw new Error('包含不受支持的日志捕获字段。');
  const mode = values.row_mode;
  if (!['manual', 'days', 'forever'].includes(mode)) throw new Error('请选择有效的日志行保留策略。');
  const settings = {
    ...original,
    enabled: values.enabled === true,
    capture_fields: [...new Set(selected)],
    body_retention_hours: integer(values.body_retention_hours, '原始内容保留小时'),
    row_retention: { ...(original.row_retention || {}), mode, days: mode === 'days' ? integer(values.row_days, '日志行保留天数', 1) : original.row_retention?.days ?? 30 },
    limits: { ...(original.limits || {}), per_request_bytes: unitsToBytes(values.per_request_kib, 'KiB', '单请求捕获上限'), total_body_bytes: unitsToBytes(values.total_body_mib, 'MiB', '原始内容磁盘预算') },
  };
  if (settings.limits.per_request_bytes < 1 || settings.limits.per_request_bytes > 16777216) throw new Error('单请求捕获上限必须为 1 字节至 16384 KiB；0 不是无限制。');
  if (mode === 'days' && settings.row_retention.days > 36500) throw new Error('日志行保留天数不能超过 36500。');
  if (runtimeManaged) {
    settings.runtime = { ...(original.runtime || {}), file_max_bytes: unitsToBytes(values.runtime_file_mib, 'MiB', '运行日志单文件上限'), retained_files: integer(values.runtime_files, '运行日志保留文件数', 1) };
    if (settings.runtime.file_max_bytes < 1024 || settings.runtime.file_max_bytes > 1073741824) throw new Error('运行日志单文件上限必须为 1 KiB 至 1024 MiB。');
    if (settings.runtime.retained_files > 32) throw new Error('运行日志文件总数不能超过 32（包括当前文件）。');
  }
  else delete settings.runtime;
  return settings;
}
export function captureExplanation(settings, backendEnabled = true) {
  if (!backendEnabled) return '日志存储后端当前不可用。页面不会把未存储的内容标为可查看。';
  if (!settings.enabled) return '请求日志已关闭，不记录新的请求日志；已有数据仍按保留策略处理。';
  if (settings.body_retention_hours === 0) return '原始内容保留为 0 小时：只记录摘要，不保存请求 / 响应的原始头和体。';
  if (settings.limits?.total_body_bytes === 0) return '原始内容磁盘预算为 0：没有原始内容存储容量，已有内容会按预算淘汰；日志摘要仍保留。';
  if (!settings.capture_fields?.length) return '当前未选择任何捕获字段，仅记录请求摘要。';
  return `已选择 ${settings.capture_fields.length} 个原始字段，内容受保留时间与大小预算共同限制。`;
}

export async function mountLogSettings(root, { signal } = {}) {
  const heading = `<div class="page-heading"><h1>日志与隐私</h1><div class="page-actions"><a class="button" href="#logs">${icon('file')}查看请求日志</a><button class="button" type="button" data-log-settings-refresh>${icon('refresh')}重新载入</button></div></div>`;
  let data;
  const live = () => !signal?.aborted;
  async function load() {
    root.innerHTML = heading + settingsNav('log-settings') + loading();
    bindRefresh();
    try { data = await api.get('/log-settings', { signal }); if (!live()) return; renderForm(); }
    catch (error) { if (live()) { root.innerHTML = heading + settingsNav('log-settings') + `<section class="card">${failure(error).replace('data-action="refresh"', 'data-log-settings-refresh')}</section>`; bindRefresh(); } }
  }
  function bindRefresh() { root.querySelectorAll('[data-log-settings-refresh]').forEach(button => button.addEventListener('click', load)); }
  function renderForm() {
    const settings = data.settings;
    const selected = new Set(settings.capture_fields || []), managed = data.runtime_rotation_managed === true;
    const initializationError = typeof data.status?.initialization_error === 'string' ? data.status.initialization_error : '';
    root.innerHTML = heading + settingsNav('log-settings') + (initializationError ? notice(`日志存储初始化失败：${initializationError}`, 'danger') : '') +
      `<form id="log-settings-form" class="card log-settings-form"><section class="form-section"><div class="form-section-header"><h2>请求日志与捕获字段</h2><p>默认不捕获任何原始内容。开启字段前，请确认应用的数据与隐私要求。</p></div>${checkbox('logging_enabled', '启用请求日志', settings.enabled === true)}<div class="capture-field-grid">${CAPTURE_FIELDS.map(item => `<label class="capture-option"><input type="checkbox" name="capture_fields" value="${item.key}" ${selected.has(item.key) ? 'checked' : ''}><span>${item.label}</span></label>`).join('')}</div><p class="capture-explanation" role="status"></p></section>` +
      `<section class="form-section"><div class="form-section-header"><h2>原始内容保留时间</h2><p>到期后可删除原始头和体；请求摘要不因此自动删除。</p></div>${field('body_retention_hours', '保留小时数', settings.body_retention_hours, { type: 'number', min: 0, step: 1, required: true, help: '0 = 不保存原始内容，不代表无限保留。' })}</section>` +
      `<section class="form-section"><div class="form-section-header"><h2>请求日志行保留策略</h2><p>日志行与原有用量 / stats 摘要是不同数据。日志行清理不会删除旧统计。</p></div><div class="field-row">${select('row_mode', '保留策略', [{ value: 'manual', label: '手工管理 · 仅手工删除' }, { value: 'days', label: '按天保留 · 自动删除到期行' }, { value: 'forever', label: '长期保留 · 不自动删除行' }], settings.row_retention?.mode)}${field('row_days', '保留天数', settings.row_retention?.days ?? '', { type: 'number', min: 1, step: 1, help: '仅“按天保留”时使用。原始内容仍受独立保留时间限制。' })}</div></section>` +
      `<section class="form-section"><div class="form-section-header"><h2>容量限制与运行日志轮转</h2><p>KiB = 1024 字节，MiB = 1048576 字节；保存时换算为精确 bytes。</p></div><div class="field-row">${field('per_request_kib', '单请求捕获上限（KiB）', bytesToUnits(settings.limits?.per_request_bytes, 'KiB'), { type: 'number', min: 1 / 1024, step: 'any', required: true, help: '范围 1 字节至 16384 KiB，0 不表示无限制。超限结果在详情中说明。' })}${field('total_body_mib', '全部原始内容磁盘预算（MiB）', bytesToUnits(settings.limits?.total_body_bytes, 'MiB'), { type: 'number', min: 0, step: 'any', required: true, help: '0 = 没有原始内容存储容量，不是无限制；淘汰后不可下载。' })}</div>${managed ? `<div class="field-row">${field('runtime_file_mib', '运行日志单文件上限（MiB）', bytesToUnits(settings.runtime?.file_max_bytes, 'MiB'), { type: 'number', min: 1 / 1024, step: 'any', required: true, help: '范围 1 KiB 至 1024 MiB。' })}${field('runtime_files', '运行日志保留文件数', settings.runtime?.retained_files, { type: 'number', min: 1, step: 1, required: true, help: '1–32 个，包含当前正在写入的文件。' })}</div>` : `<div class="runtime-unmanaged">${icon('info')}<div><strong>运行日志轮转由部署环境管理</strong><p>当前后端未声明可管理文件轮转，不提供无效开关。请通过服务管理器、容器或日志采集配置调整。</p></div></div>`}</section>` +
      `<div class="log-settings-feedback" role="status" hidden></div><div class="form-footer"><span class="hint">${data.backend_enabled === false ? '日志后端未启用，当前不能保存配置' : '保存后持久化并热生效，无需重启'}</span><button class="button primary" type="submit">${icon('check')}保存日志设置</button></div></form>`;
    bindRefresh();
    const form = root.querySelector('#log-settings-form');
    function values() {
      return { enabled: form.elements.logging_enabled.checked, capture_fields: [...form.querySelectorAll('[name="capture_fields"]:checked')].map(input => input.value), body_retention_hours: form.elements.body_retention_hours.value, row_mode: form.elements.row_mode.value, row_days: form.elements.row_days.value, per_request_kib: form.elements.per_request_kib.value, total_body_mib: form.elements.total_body_mib.value, runtime_file_mib: form.elements.runtime_file_mib?.value, runtime_files: form.elements.runtime_files?.value };
    }
    function updateState() {
      const mode = form.elements.row_mode.value;
      form.elements.row_days.disabled = mode !== 'days'; form.elements.row_days.required = mode === 'days';
      const current = values();
      root.querySelector('.capture-explanation').textContent = captureExplanation({ ...current, body_retention_hours: current.body_retention_hours.trim() === '' ? undefined : Number(current.body_retention_hours), limits: { total_body_bytes: current.total_body_mib.trim() === '' ? undefined : Number(current.total_body_mib) * 1048576 } }, data.backend_enabled !== false);
    }
    form.addEventListener('change', updateState); form.elements.body_retention_hours.addEventListener('input', updateState); form.elements.total_body_mib.addEventListener('input', updateState); updateState();
    form.elements.row_days.max = '36500'; form.elements.per_request_kib.max = '16384';
    if (managed) { form.elements.runtime_file_mib.max = '1024'; form.elements.runtime_files.max = '32'; }
    if (data.backend_enabled === false) form.querySelectorAll('input, select, button[type="submit"]').forEach(control => { control.disabled = true; });
    form.addEventListener('submit', async event => {
      event.preventDefault(); if (data.backend_enabled === false || form.dataset.busy || !form.reportValidity()) return;
      const output = form.querySelector('.log-settings-feedback'), button = form.querySelector('[type="submit"]'); output.hidden = true;
      try {
        const settings = buildLogSettings(data.settings, values(), managed);
        form.dataset.busy = 'true'; form.setAttribute('aria-busy', 'true'); button.disabled = true;
        await api.request('/log-settings', { method: 'PUT', body: { revision: data.revision, settings }, signal });
        if (!live()) return;
        data = await api.get('/log-settings', { signal });
        if (!live()) return;
        renderForm(); const feedback = root.querySelector('.log-settings-feedback'); feedback.textContent = '日志设置已持久化并热生效。已有内容的状态以服务端实际保留 / 清理结果为准。'; feedback.className = 'log-settings-feedback notice success'; feedback.hidden = false;
      } catch (error) {
        if (!live()) return;
        output.className = 'log-settings-feedback notice danger'; output.textContent = error.status === 409 ? `${error.message} 当前表单草稿仍保留；请点击“重新载入”获取最新设置后再修改。` : error.message; output.hidden = false; output.setAttribute('role', 'alert'); output.tabIndex = -1; output.focus();
      } finally { delete form.dataset.busy; form.removeAttribute('aria-busy'); button.disabled = false; }
    });
  }
  await load();
}
