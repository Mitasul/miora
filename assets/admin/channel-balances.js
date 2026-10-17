import * as api from './api.js';
import { icon } from './icons.js';
import { esc, fmt, stamp, badge, notice, dialog } from './ui.js';

const errors = {
  unsupported: '渠道未提供余额查询能力', disabled: '已关闭余额查询', no_key: '未配置凭据',
  auth_error: 'OAuth 授权不可用，请重新登录或导入账号', access_denied: '上游拒绝查询，请检查凭据与权限',
  timeout: '上游查询超时', network_error: '无法连接上游', rate_limited: '上游限制查询频率',
  plugin_error: '插件额度查询失败，请查看插件状态', busy: '查询繁忙，请稍后重试',
  plugin_unavailable: '余额插件未安装、已停用或匹配冲突，请在渠道中选择插件',
  invalid_response: '上游未返回可识别的余额数据', upstream_error: '上游返回错误',
  redirect_blocked: '上游要求重定向，未转发凭据', unsupported_credential: '该凭据不支持此查询方式',
  proxy_error: '代理配置不可用', query_error: '无法建立查询连接', response_too_large: '上游响应超过大小限制',
};
const statusText = status => errors[status] || `查询失败（${status || '未知状态'}）`;
const numeric = value => typeof value === 'number' && Number.isFinite(value);
const planName = value => ({ plus: 'Plus', pro: 'Pro', free: 'Free', team: 'Team', business: 'Business' })[value] || value;
const amountText = value => value.unlimited === true ? '无限额' : numeric(value.amount) ? `${fmt(value.amount)} ${value.currency || ''}`.trim() : '';
function windowLabel(window) {
  const seconds = window.limit_seconds;
  const duration = !numeric(seconds) || seconds <= 0 ? '' : seconds % 86400 === 0 ? `${fmt(seconds / 86400)} 天` : seconds % 3600 === 0 ? `${fmt(seconds / 3600)} 小时` : `${fmt(seconds)} 秒`;
  return [window.label || window.window || '额度', duration].filter(Boolean).join(' · ');
}
function windowValue(window, currency = '') {
  if (numeric(window.remaining_percent)) return `剩余 ${fmt(window.remaining_percent)}%`;
  if (numeric(window.remaining)) return `剩余 ${fmt(window.remaining)} ${window.unit || currency}`.trim();
  return '剩余额度未知';
}
function snapshotWarning(snapshot) {
  return snapshot?.configuration_changed ? notice('配置已更改，以下是此前查询结果，请重新获取。', 'warning') : '';
}
export function balanceSummary(snapshot) {
  if (!snapshot) return '<span class="hint">未获取</span>';
  const keys = snapshot.keys || [];
  const rows = keys.map(key => {
    const label = `账号 ${key.position}`;
    const values = key.status === 'ok' ? [planName(key.plan), amountText(key), ...(key.windows || []).slice(0, 2).map(window => `${windowLabel(window)} ${windowValue(window, key.currency)}`)].filter(Boolean) : [statusText(key.status)];
    return `<div class="channel-balance-summary" title="${esc(`${label} · 获取于 ${stamp(key.checked_at)}`)}"><strong>${esc(keys.length > 1 ? label : '')}${key.plan ? ` ${badge(planName(key.plan))}` : ''}</strong><span>${esc(values.filter(value => value !== planName(key.plan)).join(' · ') || (key.status === 'ok' ? '额度未知' : statusText(key.status)))}</span>${key.last_error ? '<span class="balance-stale">刷新失败，保留上次结果</span>' : ''}</div>`;
  }).join('');
  return `${snapshot.configuration_changed ? '<span class="balance-stale">配置已更改，待重新获取</span>' : ''}${snapshot.last_error ? '<span class="balance-stale">刷新失败，保留上次结果</span>' : ''}${rows || `<span class="hint">${esc(statusText(snapshot.status))}</span>`}`;
}
export function balanceDetails(snapshot) {
  if (!snapshot) return '<p class="hint">尚未获取余额或套餐额度。</p>';
  const keys = snapshot.keys || [];
  const rows = keys.map(key => {
    const title = `<header><strong>账号 ${esc(key.position)}</strong>${key.plan ? badge(planName(key.plan)) : ''}${key.active === false ? badge('套餐未生效 / 已到期', 'orange') : ''}${key.available === false ? badge('余额不可用', 'orange') : ''}</header>`;
    if (key.status !== 'ok') return `<article class="balance-account">${title}${notice(statusText(key.status), 'warning')}<span class="hint">尝试于 ${esc(stamp(key.checked_at))}</span></article>`;
    const windows = (key.windows || []).map(window => `<div class="balance-window"><div><span>${esc(windowLabel(window))}</span><strong>${esc(windowValue(window, key.currency))}</strong></div>${numeric(window.remaining_percent) ? `<meter min="0" max="100" value="${Math.max(0, Math.min(100, window.remaining_percent))}" aria-label="${esc(`${windowLabel(window)} ${windowValue(window)}`)}"></meter>` : ''}${window.limit_reached ? '<span class="balance-stale">已达上游限额</span>' : ''}${window.reset_at ? `<small>重置时间 ${esc(stamp(window.reset_at))}</small>` : ''}</div>`).join('');
    const buckets = (key.buckets || []).map(bucket => `<span>${esc(bucket.label)}：${esc(fmt(bucket.amount))} ${esc(bucket.unit)}</span>`).join('');
    return `<article class="balance-account">${title}${key.account_id ? `<span class="hint">${esc(key.account_id)}</span>` : ''}${amountText(key) ? `<strong class="balance-amount">${esc(amountText(key))}</strong>` : ''}${buckets ? `<div class="balance-buckets">${buckets}</div>` : ''}${windows}${!amountText(key) && !windows && !buckets ? '<p class="hint">上游未返回剩余额度。</p>' : ''}${key.expires_at ? `<span class="hint">套餐到期 ${esc(stamp(key.expires_at))}</span>` : ''}${key.last_error ? notice(`${statusText(key.last_error)}；保留上次成功结果。最近尝试 ${stamp(key.last_attempt_at)}`, 'warning') : ''}<footer class="hint">获取于 ${esc(stamp(key.checked_at))}</footer></article>`;
  }).join('');
  return snapshotWarning(snapshot) + (snapshot.last_error ? notice(`${statusText(snapshot.last_error)}；保留上次成功结果。`, 'warning') : '') + (rows || notice(statusText(snapshot.status), 'warning')) + (snapshot.omitted_keys ? notice(`另有 ${snapshot.omitted_keys} 个账号未查询，本次最多查询 8 个。`, 'warning') : '');
}
export function mountChannelBalance(container, provider, { initial, refreshOnOpen = false, onUpdated = () => {} } = {}) {
  let snapshot = initial ?? null, controller, closed = false;
  container.innerHTML = `<div class="parameter-section-heading"><h3>余额与套餐</h3><button type="button" class="button small" data-balance-refresh>${icon('refresh')}获取余额</button></div><p class="hint">${esc(provider)} · 手动获取，保留最近结果</p><div data-balance-error role="alert" hidden></div><div data-balance-content aria-live="polite">${balanceDetails(snapshot)}</div>`;
  const button = container.querySelector('[data-balance-refresh]');
  const output = container.querySelector('[data-balance-content]');
  const errorBox = container.querySelector('[data-balance-error]');
  async function load(refresh) {
    controller?.abort();
    const request = controller = new AbortController();
    button.disabled = refresh;
    errorBox.hidden = true;
    try {
      const result = await api.request(`/channels/balances${refresh ? '' : `?provider=${encodeURIComponent(provider)}`}`, { method: refresh ? 'POST' : 'GET', ...(refresh ? { body: { provider }, timeoutMs: 60000 } : {}), signal: request.signal });
      if (closed || request.signal.aborted) return;
      snapshot = refresh ? result : result.balances[provider] || null;
      output.innerHTML = balanceDetails(snapshot);
      if (refresh) onUpdated(snapshot);
    } catch (error) {
      if (closed || request.signal.aborted) return;
      errorBox.innerHTML = notice(`${error.message}；已有结果未清除。`, 'danger');
      errorBox.hidden = false;
    } finally { if (!closed && !request.signal.aborted) button.disabled = false; }
  }
  button.addEventListener('click', () => load(true));
  if (refreshOnOpen) load(true);
  else if (initial === undefined) load(false);
  return { refresh: () => load(true), destroy() { closed = true; controller?.abort(); } };
}
export function openChannelBalance(provider, initial, onUpdated) {
  let panel;
  dialog({ title: `余额与套餐 · ${provider}`, submitLabel: '关闭', body: '<section data-channel-balance></section>', onMount: form => { panel = mountChannelBalance(form.querySelector('[data-channel-balance]'), provider, { initial: initial ?? null, refreshOnOpen: true, onUpdated }); }, onSubmit: async () => {}, onClose: () => panel?.destroy() });
}
