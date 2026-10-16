import * as api from './api.js';
import { icon } from './icons.js';
import { esc, stamp, select, field, failure } from './ui.js';
import { logsNav } from './log-settings.js';

export function mergeLogEntries(previous, incoming, limit = 500) {
  return [...new Map([...previous, ...incoming].map(entry => [entry.id, entry])).values()].sort((left, right) => left.id - right.id).slice(-limit);
}
export const backendLogQuery = (after, level, query) => new URLSearchParams({ after: String(after), level, q: query, limit: '500' });

export async function mountBackendLogs(root, { signal }) {
  let items = [], cursor = 0, timer, requestController, paused = false, level = '', query = '', generation = 0;
  root.innerHTML = `<div class="page-heading"><h1>后台日志</h1><div class="page-actions"><button class="button" id="backend-refresh" type="button">${icon('refresh')}刷新</button><button class="button" id="backend-pause" type="button">暂停</button></div></div>${logsNav('backend-logs')}<section class="card"><form class="toolbar" id="backend-filter">${select('level', '级别', ['', 'DEBUG', 'INFO', 'WARN', 'ERROR', 'CRITICAL'].map(value => ({ value, label: value || '全部级别' })))}${field('q', '搜索', '', { placeholder: '事件、消息或 Request ID' })}<button class="button" type="submit">筛选</button><label class="checkbox-field"><input type="checkbox" id="backend-follow" checked>跟随最新</label></form><div id="backend-log-error" role="alert"></div><div id="backend-console" class="backend-console" tabindex="0" role="region" aria-label="运行日志"></div><div class="table-footer" id="backend-log-status" role="status"></div></section>`;
  const consoleView = root.querySelector('#backend-console');
  const error = root.querySelector('#backend-log-error');
  const status = root.querySelector('#backend-log-status');
  const pause = root.querySelector('#backend-pause');
  function draw() {
    consoleView.innerHTML = items.length ? items.map(entry => `<article class="backend-log-row" data-level="${esc(entry.level)}"><time>${esc(stamp(entry.at_ms))}</time><span class="backend-log-level">${esc(entry.level)}</span><div><strong>${esc(entry.target)}</strong><p>${esc(entry.message)}</p>${Object.keys(entry.details || {}).length ? `<details><summary>事件字段</summary><pre>${esc(JSON.stringify(entry.details, null, 2))}</pre></details>` : ''}</div></article>`).join('') : '<div class="empty-inline">当前筛选下暂无运行日志</div>';
    if (root.querySelector('#backend-follow').checked) consoleView.scrollTop = consoleView.scrollHeight;
  }
  function schedule() {
    clearTimeout(timer);
    if (!signal.aborted && !paused && !document.hidden) timer = setTimeout(() => load(false), 3000);
  }
  async function load(reset) {
    clearTimeout(timer); requestController?.abort(); requestController = new AbortController();
    const token = ++generation;
    error.textContent = '';
    try {
      const data = await api.get(`/backend-logs?${backendLogQuery(reset ? 0 : cursor, level, query)}`, { signal: requestController.signal });
      if (signal.aborted || token !== generation) return;
      const lost = !reset && cursor > 0 && data.oldest_id > cursor + 1;
      if (reset || data.items.length) { items = mergeLogEntries(reset ? [] : items, data.items); draw(); }
      cursor = data.max_id;
      status.textContent = `${paused ? '已暂停' : '每 3 秒更新'} · 显示 ${items.length} 条 · 内存保留最近 ${data.capacity} 条，重启清空${lost ? ' · 较早日志已滚动移出缓冲区' : ''}`;
    } catch (cause) { if (!signal.aborted && token === generation && !requestController.signal.aborted) { error.innerHTML = failure(cause); paused = true; pause.textContent = '继续'; } }
    finally { if (token === generation) schedule(); }
  }
  root.querySelector('#backend-filter').addEventListener('submit', event => {
    event.preventDefault(); level = event.target.elements.level.value; query = event.target.elements.q.value.trim(); load(true);
  }, { signal });
  root.querySelector('#backend-refresh').addEventListener('click', () => load(true), { signal });
  pause.addEventListener('click', () => { paused = !paused; pause.textContent = paused ? '继续' : '暂停'; if (paused) { clearTimeout(timer); status.textContent = '已暂停自动更新'; } else load(false); }, { signal });
  document.addEventListener('visibilitychange', () => { if (document.hidden) { clearTimeout(timer); requestController?.abort(); } else if (!paused) load(false); }, { signal });
  signal.addEventListener('abort', () => { clearTimeout(timer); requestController?.abort(); }, { once: true });
  await load(true);
}
