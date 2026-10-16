import * as api from './api.js';
import { icon, aiIcon, channelBrand } from './icons.js';
import { esc, badge, dialog, toast, empty } from './ui.js';

const FILE_LIMIT = 256 * 1024;
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);

export function parsePluginFile(text) {
  let value;
  try { value = JSON.parse(text); } catch { throw new Error('插件文件不是有效的 JSON。'); }
  if (!object(value) || value.format !== 'miora.plugin.v1') throw new Error('不支持此插件格式，请使用 Miora 原生插件文件。');
  const manifest = value.manifest;
  if (!object(manifest) || ['id', 'name', 'version'].some(key => typeof manifest[key] !== 'string' || !manifest[key].trim())) throw new Error('插件缺少有效的 ID、名称或版本。');
  return value;
}

export function pluginList(plugins, group = 'installed') {
  const installed = plugins.filter(plugin => plugin.kind === 'service');
  const builtins = plugins.filter(plugin => plugin.kind !== 'service');
  const visible = group === 'builtin' ? builtins : installed;
  const tabs = `<nav class="section-nav" aria-label="插件分类"><button type="button" data-action="plugin-group" data-group="builtin" ${group === 'builtin' ? 'aria-current="page"' : ''}>内置渠道插件 <span>${builtins.length}</span></button><button type="button" data-action="plugin-group" data-group="installed" ${group === 'installed' ? 'aria-current="page"' : ''}>外部扩展插件 <span>${installed.length}</span></button></nav>`;
  if (!visible.length) return tabs + `<section class="card">${empty('暂无外部插件', '可上传 .miora.json 扩展插件文件。', 'plugin')}</section>`;
  return tabs + `<section class="card plugin-list" aria-label="${group === 'builtin' ? '内置渠道插件' : '已安装插件'}">${visible.map(plugin => {
    const index = plugins.indexOf(plugin);
    const action = (label, name, glyph, primary = false) => `<button class="button small ${primary ? 'primary' : ''}" type="button" data-action="${name}" data-index="${index}">${icon(glyph)}${label}</button>`;
    const authMode = plugin.manifest?.auth_mode;
    const authBadge = authMode === 'oauth' ? badge('OAuth', 'purple') : authMode === 'oauth_or_key' ? badge('OAuth / API Key', 'purple') : plugin.kind === 'builtin' ? badge('API Key') : '';
    const desc = plugin.manifest?.description ? `<p class="subline">${esc(plugin.manifest.description)}</p>` : '';
    return `<article class="plugin-list-item"><span class="row-icon ai-brand-mark">${aiIcon(channelBrand({ provider: plugin.manifest?.name, engine: plugin.manifest?.engine, base_url: plugin.manifest?.default_base_url }), 'plugin')}</span><div class="plugin-list-name"><h2>${esc(plugin.manifest?.name || plugin.id)}</h2><code>${esc(plugin.id)}</code>${desc}</div><span class="plugin-list-version">v${esc(plugin.manifest?.version || '—')}</span>${authBadge}${badge(plugin.enabled ? '已启用' : '已停用', plugin.enabled ? 'green' : 'orange')}<div class="table-actions">${plugin.kind === 'service' ? action('更新', 'upload-plugin-update', 'upload') + action(plugin.enabled ? '停用' : '启用', 'toggle-plugin', 'power') : action('创建渠道', 'create-channel-from-plugin', 'plus', true)}${action('版本', 'plugin-detail', 'clock')}</div></article>`;
  }).join('')}</section>`;
}

export function openPluginUpload(plugins, refresh, target = null) {
  let selected = null;
  let packageData = null;
  let previous = null;
  let selection = 0;
  let active = true;
  dialog({ title: target ? `更新插件 · ${target.id}` : '上传插件', submitLabel: target ? '安装新版本' : '安装插件', size: 'plugin-upload-dialog', body:
    `<label class="upload-drop-zone" id="plugin-drop-zone" for="plugin-file">${icon('upload')}<strong>选择或拖入插件文件</strong><span id="plugin-file-help">.miora.json · 最大 256 KiB</span><input id="plugin-file" name="plugin_file" class="file-picker" type="file" accept=".miora.json,.json,application/json" aria-describedby="plugin-file-help"></label><div id="plugin-file-error" class="notice danger" role="alert" hidden></div><div id="plugin-file-preview" hidden></div>`,
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
        selected = null; packageData = null; previous = null;
        preview.hidden = true; preview.innerHTML = ''; errorBox.hidden = true; submit.disabled = true;
        form.querySelector('.dialog-error').hidden = true;
        try {
          if (files.length !== 1) throw new Error('一次只能上传一个插件文件。');
          const file = files[0];
          if (file.size > FILE_LIMIT) throw new Error('插件文件不能超过 256 KiB。');
          const parsed = parsePluginFile(await file.text());
          if (!active || current !== selection) return;
          if (target && parsed.manifest.id !== target.id) throw new Error(`请选择 ${target.id} 的更新文件。`);
          previous = plugins.find(plugin => plugin.id === parsed.manifest.id) || null;
          preview.innerHTML = `<div class="upload-package-heading"><span class="row-icon ai-brand-mark">${aiIcon(channelBrand({ provider: plugin.manifest?.name, engine: plugin.manifest?.engine, base_url: plugin.manifest?.default_base_url }), 'plugin')}</span><div><h3>${esc(parsed.manifest.name)}</h3><code>${esc(parsed.manifest.id)} · v${esc(parsed.manifest.version)}</code></div>${badge(previous ? '更新' : '新安装')}</div><dl class="definition-list upload-package-details"><div><dt>文件</dt><dd>${esc(file.name)}</dd></div><div><dt>当前版本</dt><dd>${esc(previous?.manifest?.version || '未安装')}</dd></div></dl>`;
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
      try { await api.upload('/plugins/upload', selected, { expected_revision: previous?.revision || 0 }); }
      finally { form.elements.plugin_file.disabled = false; }
      toast(previous ? '插件已更新' : '插件已安装'); await refresh();
    },
    onClose: () => { active = false; selection++; selected = null; packageData = null; previous = null; },
  });
}
