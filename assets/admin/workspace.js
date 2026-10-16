import * as api from './api.js';
import { icon } from './icons.js';
import { esc, notice, failure, loading, toast } from './ui.js';

let dirty = false;
export function confirmWorkspaceLeave() { return !dirty || window.confirm('源配置尚未保存，确认放弃修改？'); }
export const parentPath = path => path.split('/').slice(0, -1).join('/');

export async function mountWorkspace(root, { signal }) {
  let directory = '', current = null, sequence = 0, requestController;
  dirty = false;
  root.innerHTML = `<div class="page-heading"><h1>工作区</h1><button class="button" type="button" data-workspace="refresh">${icon('refresh')}刷新</button></div><div class="workspace-layout"><section class="card workspace-files"><nav class="workspace-path" aria-label="目录"><button class="button small" data-workspace="home">根目录</button><button class="button small" data-workspace="up">上一级</button><code id="workspace-directory">/</code></nav><div id="workspace-files">${loading()}</div></section><section class="card workspace-editor"><div class="card-header"><h2 id="workspace-file-name">选择文件</h2><span id="workspace-file-state" class="hint"></span></div><div id="workspace-warning"></div><label class="sr-only" for="workspace-content">文件内容</label><textarea id="workspace-content" class="code-editor" spellcheck="false" readonly placeholder="从左侧选择文件"></textarea><div id="workspace-error" role="alert"></div><div class="form-footer"><span class="hint" id="workspace-save-state" role="status"></span><button class="button" type="button" data-workspace="download" disabled>${icon('down')}下载</button><button class="button" type="button" data-workspace="revert" disabled>放弃修改</button><button class="button primary" type="button" data-workspace="save" disabled>${icon('check')}保存并生效</button></div></section></div>`;
  const editor = root.querySelector('#workspace-content');
  const error = root.querySelector('#workspace-error');
  const status = root.querySelector('#workspace-save-state');
  const save = root.querySelector('[data-workspace="save"]');
  function changed() {
    dirty = Boolean(current?.writable && editor.value !== current.content);
    save.disabled = !dirty;
    root.querySelector('[data-workspace="revert"]').disabled = !dirty;
    status.textContent = dirty ? '未保存' : '';
  }
  async function open(path = '') {
    if (path === 'config.yaml' && !window.confirm('源配置包含完整 API Key 等敏感信息，确认打开？')) return;
    requestController?.abort(); requestController = new AbortController();
    const token = ++sequence;
    error.textContent = '';
    try {
      const data = await api.get(`/workspace?${new URLSearchParams({ path })}`, { signal: requestController.signal });
      if (signal.aborted || token !== sequence) return;
      if (data.entries) {
        directory = path;
        root.querySelector('#workspace-directory').textContent = `/${path}`;
        root.querySelector('[data-workspace="up"]').disabled = !path;
        root.querySelector('#workspace-files').innerHTML = data.entries.length ? data.entries.map(entry => `<button class="workspace-entry" type="button" data-file-path="${esc(entry.path)}">${icon(entry.directory ? 'box' : 'file')}<span>${esc(entry.name)}</span>${entry.writable ? icon('edit') : ''}${entry.directory ? icon('chevron') : ''}</button>`).join('') : '<p class="empty-inline">空目录</p>';
      } else {
        if (!confirmWorkspaceLeave()) return;
        current = data; editor.value = data.content; editor.readOnly = !data.writable;
        root.querySelector('#workspace-file-name').textContent = path;
        root.querySelector('#workspace-file-state').textContent = data.writable ? '可编辑 · YAML / JSON' : '只读';
        root.querySelector('#workspace-warning').innerHTML = data.sensitive ? notice('此文件含敏感配置，不会写入浏览器存储。保存会直接更新源配置。', 'warning') : '';
        root.querySelector('[data-workspace="download"]').disabled = false;
        dirty = false; changed();
      }
    } catch (cause) { if (!signal.aborted && token === sequence && !requestController.signal.aborted) error.innerHTML = failure(cause); }
  }
  editor.addEventListener('input', changed, { signal });
  const beforeUnload = event => { if (dirty) { event.preventDefault(); event.returnValue = ''; } };
  window.addEventListener('beforeunload', beforeUnload, { signal });
  signal.addEventListener('abort', () => { dirty = false; requestController?.abort(); editor.value = ''; current = null; }, { once: true });
  root.addEventListener('click', async event => {
    const file = event.target.closest('[data-file-path]');
    if (file) { await open(file.dataset.filePath); return; }
    const button = event.target.closest('[data-workspace]');
    if (!button || button.disabled) return;
    const action = button.dataset.workspace;
    if (action === 'home') return open('');
    if (action === 'up') return open(parentPath(directory));
    if (action === 'refresh') return open(directory);
    if (action === 'revert') { if (confirmWorkspaceLeave()) { editor.value = current.content; changed(); } return; }
    if (action === 'download') {
      const url = URL.createObjectURL(new Blob([editor.value], { type: 'text/plain;charset=utf-8' }));
      const link = document.createElement('a'); link.href = url; link.download = current.path.split('/').at(-1); link.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000); return;
    }
    if (action === 'save') {
      const content = editor.value;
      button.disabled = true; editor.readOnly = true; error.textContent = '';
      try {
        const result = await api.request('/workspace', { method: 'PUT', body: { path: current.path, content, revision: current.revision }, signal });
        if (signal.aborted) return;
        current.content = content; current.revision = result.revision; changed(); status.textContent = '已保存到源文件'; toast('源配置已保存并生效');
      } catch (cause) { if (!signal.aborted) error.innerHTML = failure(cause); }
      finally { if (!signal.aborted) { editor.readOnly = false; save.disabled = !dirty; } }
    }
  }, { signal });
  await open();
}
