import * as api from './api.js';
import { icon, aiIcon, channelBrand } from './icons.js';
import { esc, badge, dialog, toast, empty } from './ui.js';

const FILE_LIMIT = 256 * 1024;
const PACKAGE_LIMIT = 16 * 1024 * 1024;
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const kindLabel = kind => ({ builtin: '内置', managed: '托管 Python', service: '独立服务' })[kind] || kind;
const statusLabel = plugin => ({ running: '运行中', disabled: '已禁用', failed: '加载失败', updating: '更新中' })[plugin.status] || (plugin.enabled ? '已启用' : '已停用');

export function pluginChanges(previous, next) {
  const before = previous?.manifest || {};
  const added = (values, old) => (values || []).filter(value => !(old || []).includes(value));
  const fields = next.manifest.config_schema || [];
  const oldFields = before.config_schema || [];
  return {
    dependencies: added(next.requirements, previous?.requirements),
    permissions: added(next.manifest.permissions, before.permissions),
    fields: fields.filter(field => !oldFields.some(old => old.key === field.key && JSON.stringify(old) === JSON.stringify(field))).map(field => field.key),
    requestFields: (next.manifest.request_schema || []).filter(field => !(before.request_schema || []).some(old => JSON.stringify(old) === JSON.stringify(field))).map(field => field.key),
    formats: added(next.manifest.capabilities?.request_formats || ['json'], before.capabilities?.request_formats || ['json']),
    nativeProxyChanged: JSON.stringify(next.manifest.native_proxy) !== JSON.stringify(before.native_proxy),
  };
}

export function parsePluginFile(text) {
  let value;
  try { value = JSON.parse(text); } catch { throw new Error('插件文件不是有效的 JSON。'); }
  if (!object(value) || value.format !== 'miora.plugin.v1') throw new Error('不支持此插件格式，请使用 Miora 原生插件文件。');
  const manifest = value.manifest;
  if (!object(manifest) || ['id', 'name', 'version'].some(key => typeof manifest[key] !== 'string' || !manifest[key].trim())) throw new Error('插件缺少有效的 ID、名称或版本。');
  return value;
}

export function pluginList(plugins, group = 'installed') {
  const installed = plugins.filter(plugin => plugin.kind !== 'builtin');
  const builtins = plugins.filter(plugin => plugin.kind === 'builtin');
  const visible = group === 'builtin' ? builtins : installed;
  const tabs = `<nav class="section-nav" aria-label="插件分类"><button type="button" data-action="plugin-group" data-group="builtin" ${group === 'builtin' ? 'aria-current="page"' : ''}>内置渠道插件 <span>${builtins.length}</span></button><button type="button" data-action="plugin-group" data-group="installed" ${group === 'installed' ? 'aria-current="page"' : ''}>外部扩展插件 <span>${installed.length}</span></button></nav>`;
  if (!visible.length) return tabs + `<section class="card">${empty('暂无扩展插件', '上传 .zip 托管插件或 .miora.json 独立服务描述。', 'plugin')}</section>`;
  return tabs + `<section class="card plugin-list" aria-label="${group === 'builtin' ? '内置渠道插件' : '已安装插件'}">${visible.map(plugin => {
    const index = plugins.indexOf(plugin);
    const action = (label, name, glyph, primary = false) => `<button class="button small ${primary ? 'primary' : ''}" type="button" data-action="${name}" data-index="${index}">${icon(glyph)}${label}</button>`;
    const authMode = plugin.manifest?.auth_mode;
    const authBadge = authMode === 'oauth' ? badge('OAuth', 'purple') : authMode === 'oauth_or_key' ? badge('OAuth / API Key', 'purple') : plugin.kind === 'builtin' ? badge('API Key') : '';
    const desc = plugin.manifest?.description ? `<p class="subline">${esc(plugin.manifest.description)}</p>` : '';
    const details = `<span class="subline">${esc(kindLabel(plugin.kind))}${plugin.manifest?.author ? ` · ${esc(plugin.manifest.author)}` : ''} · API ${esc(plugin.manifest?.api_version ?? '内置')} / SDK ${esc(plugin.manifest?.sdk_version ?? '内置')}</span><span class="subline">${esc((plugin.channels || []).join('、') || '未被渠道使用')}</span>`;
    return `<article class="plugin-list-item"><span class="row-icon ai-brand-mark">${aiIcon(channelBrand({ provider: plugin.manifest?.name, engine: plugin.manifest?.engine, base_url: plugin.manifest?.default_base_url }), 'plugin')}</span><div class="plugin-list-name"><h2>${esc(plugin.manifest?.name || plugin.id)}</h2><code>${esc(plugin.id)}</code>${desc}${details}${plugin.last_error ? `<p class="text-danger">${esc(plugin.last_error)}</p>` : ''}</div><span class="plugin-list-version">启用 v${esc(plugin.active_version || plugin.manifest?.version || '—')}<span class="subline">已装 v${esc(plugin.installed_version || plugin.manifest?.version || '—')} · 修订 ${esc(plugin.revision ?? '—')}</span></span>${authBadge}${badge(statusLabel(plugin), ['failed', 'disabled'].includes(plugin.status) || !plugin.enabled ? 'orange' : 'green')}<div class="table-actions">${action('创建渠道', 'create-channel-from-plugin', 'plus', true)}${plugin.kind !== 'builtin' ? action('更新', 'upload-plugin-update', 'upload') + action(plugin.enabled ? '停用' : '启用', 'toggle-plugin', 'power') + action('日志', 'plugin-logs', 'clock') : ''}${action('版本', 'plugin-detail', 'clock')}</div></article>`;
  }).join('')}</section>`;
}

export function openPluginUpload(plugins, refresh, target = null) {
  let selected = null;
  let packageData = null;
  let previous = null;
  let selection = 0;
  let active = true;
  let previewController;
  dialog({ title: target ? `更新插件 · ${target.id}` : '上传插件', submitLabel: target ? '安装新版本' : '安装插件', size: 'plugin-upload-dialog', body:
    `<label class="upload-drop-zone" id="plugin-drop-zone" for="plugin-file">${icon('upload')}<strong>选择或拖入插件文件</strong><span id="plugin-file-help">.zip ≤ 16 MiB · .miora.json ≤ 256 KiB</span><input id="plugin-file" name="plugin_file" class="file-picker" type="file" accept=".zip,.miora.json,.json,application/json,application/zip" aria-describedby="plugin-file-help"></label><div id="plugin-file-error" class="notice danger" role="alert" hidden></div><div id="plugin-file-preview" hidden></div>`,
    onMount: form => {
      const input = form.elements.plugin_file;
      const zone = form.querySelector('#plugin-drop-zone');
      const preview = form.querySelector('#plugin-file-preview');
      const errorBox = form.querySelector('#plugin-file-error');
      const submit = form.querySelector('button[type="submit"]');
      submit.disabled = true;
      const choose = async files => {
        if (form.getAttribute('aria-busy') === 'true') return;
        const current = ++selection;
        previewController?.abort();
        previewController = new AbortController();
        selected = null; packageData = null; previous = null;
        preview.hidden = true; preview.innerHTML = ''; errorBox.hidden = true; submit.disabled = true;
        form.querySelector('.dialog-error').hidden = true;
        try {
          if (files.length !== 1) throw new Error('一次只能上传一个插件文件。');
          const file = files[0];
          const managed = /\.zip$/i.test(file.name);
          if (file.size > (managed ? PACKAGE_LIMIT : FILE_LIMIT)) throw new Error(managed ? '插件包不能超过 16 MiB。' : '插件文件不能超过 256 KiB。');
          const parsed = managed ? await api.upload('/plugins/preview', file, {}, { timeoutMs: 45000, signal: previewController.signal }) : { ...parsePluginFile(await file.text()), kind: 'service' };
          if (!active || current !== selection) return;
          if (target && parsed.manifest.id !== target.id) throw new Error(`请选择 ${target.id} 的更新文件。`);
          previous = plugins.find(plugin => plugin.id === parsed.manifest.id) || null;
          const changes = pluginChanges(previous, parsed);
          const row = (label, value) => `<div><dt>${label}</dt><dd>${esc(value || '无')}</dd></div>`;
          preview.innerHTML = `<div class="upload-package-heading"><span class="row-icon ai-brand-mark">${aiIcon(channelBrand({ provider: parsed.manifest.name, engine: parsed.manifest.engine }), 'plugin')}</span><div><h3>${esc(parsed.manifest.name)}</h3><code>${esc(parsed.manifest.id)} · v${esc(parsed.manifest.version)}</code></div>${badge(previous ? '更新' : '新安装')}</div><dl class="definition-list upload-package-details">${row('类型', kindLabel(parsed.kind))}${row('作者', parsed.manifest.author)}${row('版本变化', `${previous?.manifest?.version || '未安装'} → ${parsed.manifest.version}`)}${row('兼容要求', `API ${parsed.manifest.api_version} / SDK ${parsed.manifest.sdk_version || 1}`)}${row('新增依赖', changes.dependencies.join('\n'))}${row('新增权限声明', changes.permissions.join('、'))}${row('配置变化', changes.fields.join('、'))}${row('专有参数变化', changes.requestFields.join('、'))}${row('新增请求格式', changes.formats.join('、'))}${row('原生认证代理变化', changes.nativeProxyChanged ? JSON.stringify(parsed.manifest.native_proxy || '移除原生入口') : '')}${row('更新说明', parsed.manifest.changelog)}</dl>`;
          selected = file; packageData = parsed;
          submit.textContent = previous ? '安装新版本' : '安装插件';
          preview.hidden = false; submit.disabled = false;
        } catch (error) {
          if (!active || current !== selection) return;
          input.value = ''; errorBox.textContent = error.message; errorBox.hidden = false;
        }
      };
      input.addEventListener('change', () => choose([...input.files]));
      for (const name of ['dragover', 'dragleave', 'drop']) zone.addEventListener(name, event => {
        event.preventDefault();
        zone.classList.toggle('is-dragging', name === 'dragover');
        if (name === 'drop' && form.getAttribute('aria-busy') !== 'true') choose([...event.dataTransfer.files]);
      });
    },
    onSubmit: async form => {
      if (!selected || !packageData) throw new Error('请先选择有效的插件文件。');
      form.elements.plugin_file.disabled = true;
      try { await api.upload('/plugins/upload', selected, { expected_revision: previous?.revision || 0 }, { timeoutMs: 720000 }); }
      finally { form.elements.plugin_file.disabled = false; }
      toast(previous ? '插件已更新' : '插件已安装'); await refresh();
    },
    onClose: () => { active = false; selection++; previewController?.abort(); selected = null; packageData = null; previous = null; },
  });
}

export async function openPluginLogs(plugin) {
  const result = await api.get(`/plugins/${encodeURIComponent(plugin.id)}/logs`);
  dialog({ title: `运行日志 · ${plugin.id}`, submitLabel: '关闭', body: `${result.last_error ? `<p class="text-danger">${esc(result.last_error)}</p>` : ''}<pre>${esc((result.chunks || []).join('') || '暂无进程日志；独立服务请查看其宿主日志。')}</pre>`, onSubmit: async () => {} });
}
