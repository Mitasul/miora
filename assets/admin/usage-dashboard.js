import { icon, aiIcon, channelBrand, modelBrand } from './icons.js';
import { esc, fmt, select, field, empty, notice, dialog } from './ui.js';

export const percent = value => typeof value === 'number' && Number.isFinite(value) ? `${(value * 100).toFixed(1)}%` : '—';
export const money = value => typeof value === 'number' && Number.isFinite(value) ? `$${value.toFixed(6)}` : '—';
const milliseconds = value => typeof value === 'number' && Number.isFinite(value) ? `${fmt(Math.round(value))} ms` : '—';
const coverage = (samples, count) => count > 0 && samples < count ? `<span class="subline">已报告 ${fmt(samples)} / ${fmt(count)}</span>` : '';
const tokenCell = (row, key, samples) => `${fmt(row[key])}${coverage(row[samples] || 0, row.request_count)}`;
const metric = (label, value, note = '') => `<article class="usage-summary-item"><span>${label}</span><strong>${value}</strong>${note ? `<small>${note}</small>` : ''}</article>`;
const trendMetrics = [
  { value: 'request_count', label: '请求数' }, { value: 'failure_count', label: '失败数' },
  { value: 'input_tokens', label: '输入 Token' }, { value: 'output_tokens', label: '输出 Token' },
  { value: 'cache_read_tokens', label: '缓存命中 Token' }, { value: 'cache_miss_tokens', label: '缓存未命中 Token' },
  { value: 'cache_write_tokens', label: '缓存写入 Token' }, { value: 'estimated_cost', label: '费用估算 · USD' },
];

export function usageLogLink(row, data, filters) {
  const params = new URLSearchParams();
  const model = row.model ?? filters.model;
  const channel = row.channel ?? filters.channel;
  if (model) params.set('model', model);
  if (channel) params.set('channel', channel);
  if (data.from !== undefined) params.set('from_ms', String(data.from * 1000));
  if (data.to !== undefined) params.set('to_ms', String(data.to * 1000));
  return `#logs?${params}`;
}

export function usageBreakdown(data, filters) {
  const byChannel = filters.group === 'channel';
  const rows = byChannel ? data.channels : data.usage;
  const truncated = byChannel ? data.channels_truncated : data.truncated;
  const headers = [byChannel ? '渠道' : '模型', '请求 / 失败', '输入 Token', '输出 Token', '总 Token', '缓存命中', '缓存未命中', '缓存写入', '命中率', '费用估算 · USD', '平均耗时 / 首字', ''];
  const table = rows.length ? `<div class="table-scroll" tabindex="0" role="region" aria-label="${byChannel ? '渠道' : '模型'}使用明细"><table class="model-usage-table"><thead><tr>${headers.map(label => `<th scope="col">${label}</th>`).join('')}</tr></thead><tbody>${rows.map((row, index) => {
    const name = byChannel ? row.channel : row.model;
    return `<tr><td>${name ? `<a class="model-usage-name" href="${esc(usageLogLink(row, data, filters))}">${aiIcon(byChannel ? channelBrand({ provider: name }) : modelBrand(name))}${esc(name)}</a>` : '<span class="muted">未记录渠道</span>'}${byChannel ? '' : `<span class="subline">${fmt(row.channel_count)} 个渠道</span>`}</td><td>${fmt(row.request_count)}<span class="subline">失败 ${fmt(row.failure_count)}</span></td><td>${tokenCell(row, 'input_tokens', 'input_samples')}</td><td>${tokenCell(row, 'output_tokens', 'output_samples')}</td><td>${tokenCell(row, 'total_tokens', 'total_samples')}</td><td>${tokenCell(row, 'cache_read_tokens', 'cache_read_samples')}</td><td>${tokenCell(row, 'cache_miss_tokens', 'cache_miss_samples')}</td><td>${tokenCell(row, 'cache_write_tokens', 'cache_write_samples')}</td><td>${percent(row.cache_hit_rate)}</td><td>${money(row.estimated_cost)}${coverage(row.priced_requests || 0, row.request_count)}</td><td>${milliseconds(row.average_duration_ms)}<span class="subline">首字 ${milliseconds(row.average_first_text_ms)}</span></td><td><button class="button small" type="button" data-action="usage-detail" data-index="${index}">${icon('eye')}明细</button></td></tr>`;
  }).join('')}</tbody></table></div>` : empty('所选范围内暂无请求', '', 'chart');
  return `<div class="card-header"><h2>${byChannel ? '渠道对比' : '模型明细'}</h2><span class="hint">${fmt(rows.length)} 个${byChannel ? '渠道' : '模型'}${truncated ? ' · 仅显示前 500 个' : ''}</span></div>${table}`;
}

export function usageTrend(data, filters, selected = 'request_count') {
  const rows = data.trend;
  const points = rows.filter(row => row[selected] !== null && row[selected] !== undefined);
  if (!points.length) return empty(rows.length ? '当前区间未报告该指标' : '所选范围内暂无趋势数据', '', 'chart');
  const maximum = Math.max(...points.map(row => row[selected]));
  const span = data.to - data.from;
  const label = trendMetrics.find(item => item.value === selected).label;
  const valueLabel = value => selected === 'estimated_cost' ? money(value) : fmt(value);
  const bars = points.map(row => {
    const from = Math.max(data.from, row.bucket_start);
    const to = Math.min(data.to, row.bucket_start + data.bucket_seconds);
    const position = (from - data.from) / span * 800;
    const width = Math.max(0.5, (to - from) / span * 800 - 1);
    const height = maximum === 0 ? 0 : row[selected] / maximum * 120;
    const title = `${new Date(from * 1000).toLocaleString()} – ${new Date(to * 1000).toLocaleString()} · ${label} ${valueLabel(row[selected])} · 成功 ${fmt(row.success_count)} / 失败 ${fmt(row.failure_count)}`;
    return `<a href="${esc(usageLogLink({}, { from, to }, filters))}" aria-label="${esc(title)}"><title>${esc(title)}</title><rect x="${position.toFixed(2)}" y="${(128 - height).toFixed(2)}" width="${width.toFixed(2)}" height="${height.toFixed(2)}" /></a>`;
  }).join('');
  return `<div class="usage-trend-scale"><span>${esc(valueLabel(maximum))}</span><span>${data.bucket_seconds === 3600 ? '按小时 · UTC 分桶' : '按天 · UTC 分桶'}</span></div><svg class="usage-trend-chart" viewBox="0 0 800 136" preserveAspectRatio="none" role="group" aria-label="${esc(label)}趋势，点击柱形查看该时段日志"><path d="M0 128H800" />${bars}</svg><div class="usage-trend-scale"><span>${esc(new Date(data.from * 1000).toLocaleString())}</span><span>${esc(new Date(data.to * 1000).toLocaleString())}</span></div>`;
}

export function usageDashboard(data, filters) {
  const form = `<form id="usage-filter" class="toolbar usage-toolbar"><div class="filter-fields">${select('days', '时间范围', [1, 7, 30, 90, 365].map(days => ({ value: days, label: `最近 ${days} 天` })), filters.days)}${field('model', '模型', filters.model, { placeholder: '全部模型' })}${field('channel', '渠道', filters.channel, { placeholder: '全部渠道' })}${select('group', '分组', [{ value: 'model', label: '按模型' }, { value: 'channel', label: '按渠道' }], filters.group || 'model')}</div><button type="submit" class="button">${icon('filter')}筛选</button></form>`;
  if (!data || data.available === false) return `<section class="card">${form}${empty('用量存储未启用', '启用数据库后记录模型使用数据。', 'database')}</section>`;
  const summary = data.summary || {};
  const count = summary.request_count || 0;
  const metrics = `<section class="usage-summary-grid" aria-label="模型使用统计">${metric('请求数', fmt(count), `成功 ${fmt(summary.success_count)} · 失败 ${fmt(summary.failure_count)}`)}${metric('输入 Token', fmt(summary.input_tokens), coverage(summary.input_samples || 0, count))}${metric('输出 Token', fmt(summary.output_tokens), coverage(summary.output_samples || 0, count))}${metric('缓存命中 Token', fmt(summary.cache_read_tokens), `命中率 ${percent(summary.cache_hit_rate)}`)}${metric('缓存未命中 Token', fmt(summary.cache_miss_tokens), coverage(summary.cache_miss_samples || 0, count))}${metric('缓存写入 Token', fmt(summary.cache_write_tokens), coverage(summary.cache_write_samples || 0, count))}${metric('费用估算 · USD', money(summary.estimated_cost), `已计价 ${fmt(summary.priced_requests || 0)} / ${fmt(count)} 请求`)}${metric('平均耗时', milliseconds(summary.average_duration_ms), `首字 ${milliseconds(summary.average_first_text_ms)}`)}</section>`;
  return `<section class="card model-usage-card">${form}${metrics}<section class="usage-trend"><div class="usage-trend-heading"><h2>使用趋势</h2>${select('trend_metric', '指标', trendMetrics, 'request_count')}</div><div id="usage-trend-content">${usageTrend(data, filters)}</div></section><div id="usage-breakdown">${usageBreakdown(data, filters)}</div><div class="usage-footnote">费用按渠道单价估算，不等于供应商账单。未报告字段显示 —，缓存未命中 = 已报告输入 − 缓存读取。</div></section>`;
}

export function openUsageDetail(row) {
  const count = row.request_count || 0;
  const items = [
    ['请求数', fmt(count)], ['成功 / 失败', `${fmt(row.success_count)} / ${fmt(row.failure_count)}`],
    ['输入 Token', tokenCell(row, 'input_tokens', 'input_samples')], ['输出 Token', tokenCell(row, 'output_tokens', 'output_samples')], ['总 Token', tokenCell(row, 'total_tokens', 'total_samples')],
    ['缓存命中 Token', tokenCell(row, 'cache_read_tokens', 'cache_read_samples')], ['缓存未命中 Token', tokenCell(row, 'cache_miss_tokens', 'cache_miss_samples')],
    ['缓存写入（含 1 小时）', tokenCell(row, 'cache_write_tokens', 'cache_write_samples')], ['其中 1 小时写入', tokenCell(row, 'cache_write_1h_tokens', 'cache_write_1h_samples')],
    ['缓存命中率', percent(row.cache_hit_rate)], ['费用估算 · USD', money(row.estimated_cost)], ['已计价请求', `${fmt(row.priced_requests || 0)} / ${fmt(count)}`],
    ['平均耗时', milliseconds(row.average_duration_ms)], ['平均首字耗时', milliseconds(row.average_first_text_ms)],
    ['历史输入计数（含旧记录）', fmt(row.total_prompt_tokens)], ['历史输出计数（含旧记录）', fmt(row.total_completion_tokens)],
  ];
  dialog({ title: row.channel === undefined ? row.model : row.channel || '未记录渠道', submitLabel: '关闭', body: `<dl class="definition-list">${items.map(([label, value]) => `<div><dt>${label}</dt><dd>${value}</dd></div>`).join('')}</dl>` + notice('缓存写入已包含在输入内，不重复累加。旧记录没有的缓存和价格快照不会被推测补齐。'), onSubmit: async () => {} });
}
