import { icon } from './icons.js';
export const esc = value => String(value ?? '').replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char]);
const numberFormatter = new Intl.NumberFormat('zh-CN');
export const fmt = value => typeof value === 'number' && Number.isFinite(value) ? numberFormatter.format(value) : '—';
export const stamp = value => { if (value === null || value === undefined || value === '') return '—'; const d = new Date(typeof value === 'number' ? value * 1000 : value); return Number.isNaN(d.getTime()) ? String(value) : d.toLocaleString('zh-CN', { hour12: false }); };
export const badge = (text, tone = '', name = '') => `<span class="badge ${esc(tone)}">${name ? icon(name) : ''}${esc(text)}</span>`;
export const notice = (text, tone = '') => `<div class="notice ${esc(tone)}">${icon(tone === 'warning' || tone === 'danger' ? 'alert' : 'info')}<span>${esc(text)}</span></div>`;
export const empty = (title, description, name = 'box') => `<div class="empty-state">${icon(name)}<h3>${esc(title)}</h3><p>${esc(description)}</p></div>`;
export const failure = error => `<div class="error-state" role="alert">${icon('alert')}<h3>暂时无法加载</h3><p>${esc(error.message)}</p><button class="button small" data-action="refresh">${icon('refresh')}重试</button></div>`;
export const loading = () => '<div class="loading-state" role="status"><span class="spinner"></span><p>正在读取网关数据…</p></div>';
export const jsonDetails = (value, title = '查看响应详情') => `<details class="advanced"><summary>${esc(title)}</summary><pre>${esc(JSON.stringify(value, null, 2))}</pre></details>`;
export function field(name, label, value = '', { type = 'text', required = false, help = '', placeholder = '', readonly = false, min, step, rows = 4 } = {}) {
  const id = `f-${name}`;
  const common = `id="${esc(id)}" name="${esc(name)}" ${required ? 'required' : ''} ${readonly ? 'readonly' : ''} ${help ? `aria-describedby="${esc(id)}-help"` : ''}`;
  const input = type === 'textarea' ? `<textarea ${common} rows="${rows}" placeholder="${esc(placeholder)}" spellcheck="false">${esc(value)}</textarea>` : `<input ${common} type="${esc(type)}" value="${esc(value)}" placeholder="${esc(placeholder)}" ${type === 'password' ? 'autocomplete="new-password"' : ''} ${min !== undefined ? `min="${esc(min)}"` : ''} ${step !== undefined ? `step="${esc(step)}"` : ''}>`;
  return `<div class="field"><label for="${esc(id)}">${esc(label)}${required ? '<span class="required" aria-hidden="true">*</span>' : ''}</label>${input}${help ? `<small id="${esc(id)}-help">${esc(help)}</small>` : ''}</div>`;
}
export function select(name, label, options, value = '', help = '') {
  return `<div class="field"><label for="f-${esc(name)}">${esc(label)}</label><select name="${esc(name)}" id="f-${esc(name)}">${options.map(option => `<option value="${esc(option.value)}" ${String(value) === String(option.value) ? 'selected' : ''}>${esc(option.label)}</option>`).join('')}</select>${help ? `<small>${esc(help)}</small>` : ''}</div>`;
}
export const checkbox = (name, label, checked = false) => `<div class="checkbox-field"><input id="f-${esc(name)}" name="${esc(name)}" type="checkbox" ${checked ? 'checked' : ''}><label for="f-${esc(name)}">${esc(label)}</label></div>`;
export function parseJSON(value, label, expected = 'object') {
  let data;
  try { data = JSON.parse(value); } catch { throw new Error(`${label} 不是有效的 JSON。`); }
  if (expected === 'object' && (!data || Array.isArray(data) || typeof data !== 'object')) throw new Error(`${label} 必须是 JSON 对象。`);
  if (expected === 'array' && !Array.isArray(data)) throw new Error(`${label} 必须是 JSON 数组。`);
  return data;
}
export function schemaFields(schema = [], config = {}, prefix = 'schema') {
  if (!Array.isArray(schema) || !schema.length) return '<p class="hint">此插件未声明额外配置字段。</p>';
  return schema.map((spec, index) => {
    const name = `${prefix}-${index}`, label = spec.label || spec.key;
    const value = Object.hasOwn(config || {}, spec.key) ? config[spec.key] : spec.default;
    const help = `${spec.key}${spec.required ? ' · 必填' : ' · 可选'}${spec.secret ? ' · 敏感值不回显；留空保留原值' : ''}`;
    if (spec.type === 'boolean') return checkbox(name, `${label}${spec.required ? '（必填）' : ''}`, value === true);
    if (Array.isArray(spec.enum) && spec.enum.length) return select(name, label, [{ value: '', label: '请选择' }, ...spec.enum.map((v, i) => ({ value: i, label: String(v) }))], value === undefined ? '' : spec.enum.findIndex(v => JSON.stringify(v) === JSON.stringify(value)), help);
    const complex = spec.type === 'object' || spec.type === 'array';
    return field(name, label, spec.secret ? '' : complex ? value === undefined ? '' : JSON.stringify(value, null, 2) : value ?? '', { type: spec.secret ? 'password' : complex ? 'textarea' : ['number', 'integer'].includes(spec.type) ? 'number' : 'text', required: Boolean(spec.required && !(spec.secret && value)), step: spec.type === 'integer' ? 1 : 'any', help, placeholder: spec.secret && value ? '已配置，留空保留' : '' });
  }).join('');
}
export function readSchema(form, schema = [], existing = {}, prefix = 'schema', preserveMasked = false) {
  const result = Object.create(null);
  for (const [index, spec] of schema.entries()) {
    const element = form.elements.namedItem(`${prefix}-${index}`);
    if (!element) continue;
    const raw = element.value;
    if (spec.secret && raw === '') { if (preserveMasked && Object.hasOwn(existing, spec.key)) result[spec.key] = existing[spec.key]; continue; }
    if (spec.type === 'boolean') result[spec.key] = element.checked;
    else if (raw === '') { if (spec.required) throw new Error(`请填写 ${spec.label || spec.key}。`); }
    else if (Array.isArray(spec.enum) && spec.enum.length) result[spec.key] = spec.enum[Number(raw)];
    else if (['integer', 'number'].includes(spec.type)) { const number = Number(raw); if (!Number.isFinite(number) || (spec.type === 'integer' && !Number.isInteger(number))) throw new Error(`${spec.label || spec.key} 必须是有效数字。`); result[spec.key] = number; }
    else if (spec.type === 'object' || spec.type === 'array') result[spec.key] = parseJSON(raw, spec.label || spec.key, spec.type);
    else result[spec.key] = raw;
  }
  return result;
}
let toastTimer;
export const isSaveShortcut = event => event.key === 'Enter' && (event.ctrlKey || event.metaKey) && !event.altKey && !event.shiftKey && !event.isComposing;
export function toast(message) {
  const region = document.querySelector('#toast-region');
  region.innerHTML = `<div class="toast">${icon('circleCheck')}<span>${esc(message)}</span></div>`;
  clearTimeout(toastTimer); toastTimer = setTimeout(() => { region.textContent = ''; }, 5500);
}
export function dialog({ title, body, submitLabel = '保存更改', danger = false, size = '', showCancel = true, onSubmit, onMount, onClose }) {
  const modal = document.querySelector('#editor-dialog');
  if (modal.open) modal.close();
  const previous = document.activeElement;
  modal.classList.toggle('channel-dialog', size === 'channel');
  modal.classList.toggle('request-log-dialog', size === 'request-log');
  modal.classList.toggle('plugin-upload-dialog', size === 'plugin-upload-dialog');
  document.querySelector('#dialog-content').innerHTML = `<form id="editor-form"><div class="dialog-head"><h2 id="dialog-title">${esc(title)}</h2><button class="icon-button" type="button" data-close aria-label="关闭对话框">${icon('close')}</button></div><div class="dialog-body">${body}</div><div class="notice danger dialog-error" role="alert" hidden></div><div class="form-footer">${showCancel ? '<button class="button" type="button" data-close>取消</button>' : ''}<button class="button ${danger ? 'danger' : 'primary'}" type="submit">${icon(danger ? 'alert' : 'check')}${esc(submitLabel)}</button></div></form>`;
  const form = document.querySelector('#editor-form');
  let busy = false;
  modal.oncancel = event => { if (busy) event.preventDefault(); };
  modal.onclose = () => {
    // Ignore an earlier dialog's queued close event when a new one is already open.
    if (modal.open || !modal.contains(form)) return;
    onClose?.();
    form.querySelectorAll('input, textarea').forEach(input => { input.value = ''; input.removeAttribute('value'); });
    form.remove(); modal.oncancel = null; modal.onclose = null;
    if (previous?.isConnected) previous.focus();
  };
  form.querySelectorAll('[data-close]').forEach(button => button.addEventListener('click', () => { if (!busy) modal.close(); }));
  form.querySelector('button[type="submit"]').title = 'Ctrl+Enter / Command+Enter';
  form.addEventListener('keydown', event => {
    if (!isSaveShortcut(event)) return;
    event.preventDefault();
    if (!busy) form.requestSubmit();
  });
  form.addEventListener('submit', async event => {
    event.preventDefault(); if (busy || !form.reportValidity()) return;
    busy = true; const buttons = [...form.querySelectorAll('button')]; const disabledStates = buttons.map(button => button.disabled); buttons.forEach(button => { button.disabled = true; }); form.setAttribute('aria-busy', 'true');
    const errorBox = form.querySelector('.dialog-error'); errorBox.hidden = true;
    try { await onSubmit(form); modal.close(); }
    catch (error) { errorBox.textContent = error.message; errorBox.hidden = false; errorBox.tabIndex = -1; errorBox.focus(); }
    finally { busy = false; buttons.forEach((button, index) => { button.disabled = disabledStates[index]; }); form.removeAttribute('aria-busy'); }
  });
  onMount?.(form);
  modal.showModal();
  return form;
}
export function confirmAction({ title, message, label, action }) {
  return dialog({ title, body: notice(message, 'warning'), submitLabel: label, danger: true, onSubmit: action });
}
