import * as api from './api.js';
import { icon } from './icons.js';
import { esc, fmt, badge, field, select, checkbox, notice, empty, dialog, confirmAction, toast } from './ui.js';

export const parseGroups = value => [...new Set(value.split(/[,，\n]/).map(group => group.trim()).filter(Boolean))];
export const keyModelRules = key => (key.models || ['all']).map(rule => typeof rule === 'string' ? rule : Object.keys(rule)[0]);
export function filterKeys(keys, { query = '', group = '', status = '' } = {}) {
  const search = query.trim().toLowerCase();
  return keys.filter(key => (!group || key.groups.includes(group))
    && (!status || key.enabled === (status === 'enabled'))
    && (!search || `${key.name} ${key.id} ${key.groups.join(' ')} ${keyModelRules(key).join(' ')}`.toLowerCase().includes(search)));
}
export function groupModels(channels, groups) {
  return [...new Set(channels.filter(channel => (channel.groups || ['default']).some(group => groups.includes(group))).flatMap(channel => channel.models))].sort();
}

export async function openGroupManager({ onChanged = async () => {} } = {}) {
  let keysData = await api.get('/api-keys');
  let channelsData = await api.get('/channels');
  let form;
  function allGroups() {
    return [...new Set(['default', ...(keysData.groups || []), ...keysData.api_keys.flatMap(k => k.groups || ['default']), ...(channelsData.channels || []).flatMap(c => c.groups || ['default'])])].sort();
  }
  function renderTable() {
    if (!form) return;
    const groups = allGroups();
    const savedSet = new Set(keysData.groups || []);
    const listRoot = form.querySelector('[data-group-mgr-list]');
    listRoot.innerHTML = `<div class="table-scroll"><table><thead><tr><th>分组名称</th><th>类型</th><th>关联渠道数</th><th>关联 API Key 数</th><th>操作</th></tr></thead><tbody>${groups.map(name => {
      const chCount = (channelsData.channels || []).filter(c => (c.groups || ['default']).includes(name)).length;
      const keyCount = (keysData.api_keys || []).filter(k => (k.groups || ['default']).includes(name)).length;
      const isDefault = name === 'default';
      return `<tr><td><strong><code>${esc(name)}</code></strong></td><td>${badge(isDefault ? '系统默认' : savedSet.has(name) ? '自定义分组' : '配置引用', isDefault ? '' : 'purple')}</td><td>${fmt(chCount)} 个渠道</td><td>${fmt(keyCount)} 个密钥</td><td>${!isDefault && savedSet.has(name) ? `<button type="button" class="button small text-danger" data-group-mgr-remove="${esc(name)}">${icon('trash')}删除分组</button>` : '—'}</td></tr>`;
    }).join('')}</tbody></table></div>`;
  }
  async function reload() {
    [keysData, channelsData] = await Promise.all([api.get('/api-keys'), api.get('/channels')]);
    renderTable();
    await onChanged();
  }
  form = dialog({
    title: '分组管理',
    size: 'wide',
    submitLabel: '完成',
    showCancel: false,
    body: `<section class="form-section"><div class="inline-row"><input type="text" data-group-mgr-input placeholder="输入新分组名（多个用逗号分隔，如 vip, codex）" aria-label="新分组名称"><button type="button" class="button primary" data-group-mgr-add>${icon('plus')}添加分组</button></div><p data-group-mgr-error class="text-danger" role="alert"></p></section><section class="form-section"><div data-group-mgr-list></div></section>`,
    onSubmit: async () => {},
    onMount: mounted => {
      form = mounted;
      renderTable();
      const input = form.querySelector('[data-group-mgr-input]');
      const errBox = form.querySelector('[data-group-mgr-error]');
      const doAdd = async () => {
        errBox.textContent = '';
        const names = parseGroups(input.value);
        if (!names.length) { input.focus(); return; }
        try {
          await api.patch('/api-keys', { revision: keysData.revision, group_action: 'add', group_name: names.join(', ') });
          input.value = '';
          toast(`已添加分组：${names.join(', ')}`);
          await reload();
        } catch (err) {
          errBox.textContent = err.message;
        }
      };
      form.querySelector('[data-group-mgr-add]').addEventListener('click', doAdd);
      input.addEventListener('keydown', event => { if (event.key === 'Enter' && !event.isComposing) { event.preventDefault(); doAdd(); } });
      form.addEventListener('click', async event => {
        const removeBtn = event.target.closest('[data-group-mgr-remove]');
        if (!removeBtn) return;
        errBox.textContent = '';
        const targetName = removeBtn.dataset.groupMgrRemove;
        try {
          await api.patch('/api-keys', { revision: keysData.revision, group_action: 'remove', group_name: targetName });
          toast(`已删除分组：${targetName}`);
          await reload();
        } catch (err) {
          errBox.textContent = err.message;
        }
      });
    },
  });
}

function showSecret(secret) {
  dialog({ title: 'API Key 已创建', submitLabel: '关闭', body: notice('请复制保存，关闭后密钥列表不再回显。', 'warning')
    + field('new_key', 'API Key', secret, { readonly: true })
    + `<button class="button" type="button" id="copy-key">${icon('copy')}复制密钥</button><span id="copy-key-status" class="hint" role="status"></span>`,
  onSubmit: async () => {}, onMount: form => {
    form.querySelector('#copy-key').addEventListener('click', async () => {
      try { await navigator.clipboard.writeText(form.elements.new_key.value); form.querySelector('#copy-key-status').textContent = '已复制'; }
      catch { form.elements.new_key.select(); form.querySelector('#copy-key-status').textContent = '已选中，请按 Ctrl+C 复制'; }
    });
  }, onClose: () => { secret = ''; } });
}

export async function mountKeys(root, { signal }) {
  let data, channels, knownGroups;
  const filters = new URLSearchParams(window.location.hash.split('?')[1] || '');
  let group = filters.get('group') || '', query = filters.get('q') || '', status = filters.get('status') || '';
  async function load() {
    const [keys, catalog] = await Promise.all([api.get('/api-keys', { signal }), api.get('/channels', { signal })]);
    if (signal.aborted) return;
    data = keys; channels = catalog.channels;
    knownGroups = [...new Set(['default', ...(data.groups || []), ...data.api_keys.flatMap(key => key.groups), ...channels.flatMap(channel => channel.groups || ['default'])])].sort();
    root.innerHTML = `<div class="page-heading"><h1>API Key</h1><div class="page-actions"><button class="button" type="button" data-key-action="refresh">${icon('refresh')}刷新</button>${data.writable ? `<button class="button" type="button" data-key-action="manage-groups">${icon('shield')}分组管理</button><button class="button primary" type="button" data-key-action="create">${icon('plus')}创建密钥</button>` : ''}</div></div>${data.writable ? '' : notice('远程配置只读', 'warning')}<section class="card"><div class="group-tabs" role="group" aria-label="密钥分组">${['', ...knownGroups].map(value => `<button class="button small" type="button" data-key-group="${esc(value)}" aria-pressed="${value === group}">${esc(value || '全部分组')}<span class="group-count">${fmt(value ? data.api_keys.filter(key => key.groups.includes(value)).length : data.api_keys.length)}</span></button>`).join('')}${data.writable ? `<div class="group-quick-add"><input id="quick-group-input" type="text" placeholder="+ 新建分组" aria-label="快速新建分组"><button class="button small" type="button" data-key-action="quick-add-group">${icon('plus')}添加</button>${group && group !== 'default' && (data.groups || []).includes(group) ? `<button class="button small text-danger" type="button" data-key-action="remove-group" data-group-name="${esc(group)}">${icon('trash')}删除</button>` : ''}</div>` : ''}</div><div class="toolbar"><div class="search-field">${icon('search')}<input id="key-search" type="search" placeholder="搜索名称、分组或模型" aria-label="搜索密钥" value="${esc(query)}"></div>${select('key_status', '状态', [{ value: '', label: '全部状态' }, { value: 'enabled', label: '已启用' }, { value: 'disabled', label: '已停用' }], status)}</div><div id="key-list"></div></section>`;
    root.querySelector('#key-search').addEventListener('input', event => { query = event.target.value; draw(); }, { signal });
    root.querySelector('#f-key_status').addEventListener('change', event => { status = event.target.value; draw(); }, { signal });
    root.querySelector('#quick-group-input')?.addEventListener('keydown', event => {
      if (event.key === 'Enter' && !event.isComposing) {
        event.preventDefault();
        root.querySelector('[data-key-action="quick-add-group"]')?.click();
      }
    }, { signal });
    draw();
  }
  function draw() {
    const rows = filterKeys(data.api_keys, { query, group, status });
    root.querySelectorAll('[data-key-group]').forEach(button => button.setAttribute('aria-pressed', String(button.dataset.keyGroup === group)));
    root.querySelector('#key-list').innerHTML = rows.length ? `<div class="table-scroll" tabindex="0"><table><thead><tr><th>名称 / 凭据</th><th>分组</th><th>状态</th><th>授权模型</th><th>速率限制</th><th>操作</th></tr></thead><tbody>${rows.map(key => `<tr><td><strong>${esc(key.name || `Key ${key.position}`)}</strong><span class="subline"><code>${esc(key.masked_key)}</code> · ${esc(key.role)}</span></td><td><div class="tags">${key.groups.map(value => badge(value)).join('')}</div></td><td>${badge(key.enabled ? '启用' : '停用', key.enabled ? 'green' : 'orange')}</td><td><div class="tags">${keyModelRules(key).slice(0, 4).map(model => badge(model)).join('')}${keyModelRules(key).length > 4 ? badge(`+${keyModelRules(key).length - 4}`) : ''}</div></td><td>${esc(typeof key.rate_limit === 'string' ? key.rate_limit : key.rate_limit ? '按模型配置' : '继承全局')}</td><td><div class="table-actions">${key.enabled ? `<a class="button small" href="#testing?position=${key.position}">${icon('pulse')}测试</a>` : ''}${data.writable ? `<button class="button small" type="button" data-key-action="edit" data-key-id="${esc(key.id)}">${icon('edit')}编辑</button><button class="button small" type="button" data-key-action="toggle" data-key-id="${esc(key.id)}">${icon('power')}${key.enabled ? '停用' : '启用'}</button><button class="button small text-danger" type="button" data-key-action="revoke" data-key-id="${esc(key.id)}">${icon('trash')}吊销</button>` : ''}</div></td></tr>`).join('')}</tbody></table></div><div class="table-footer">显示 ${rows.length} / ${data.api_keys.length} 个密钥</div>` : empty('没有匹配的密钥', '', 'key');
    const params = new URLSearchParams();
    if (group) params.set('group', group); if (query) params.set('q', query); if (status) params.set('status', status);
    window.history.replaceState(null, '', `#keys${params.size ? `?${params}` : ''}`);
  }
  function edit(key) {
    const originalModels = key ? keyModelRules(key).join('\n') : 'all';
    const originalRate = typeof key?.rate_limit === 'string' ? key.rate_limit : '';
    dialog({ title: key ? '编辑 API Key' : '创建 API Key', submitLabel: key ? '保存' : '创建密钥',
      body: field('name', '名称', key?.name || '', { placeholder: '例如：开发环境' })
        + `<div class="field"><label class="field-label" for="f-key_groups_input">分组</label><input id="f-groups" name="groups" type="hidden" value="${esc((key?.groups || [group || 'default']).join(', '))}"><div class="group-picker"><div class="group-chip-list" data-key-group-chips></div><input id="f-key_groups_input" type="text" class="group-inline-input" data-key-new-group placeholder="+ 输入新分组按回车" aria-label="输入新分组名"></div></div>`
        + select('role', '角色', [{ value: 'user', label: '普通 API 访问' }, { value: 'admin', label: '管理员 API 权限' }], key?.role || 'user')
        + field('models', '授权模型（每行一个）', originalModels, { type: 'textarea', required: true, help: 'all 或 渠道名/模型' })
        + '<button class="button small" type="button" id="key-load-models">载入分组模型</button>'
        + field('rate_limit', '请求速率', originalRate, { placeholder: '例如 60/min；留空继承全局' })
        + checkbox('enabled', '启用此密钥', key?.enabled ?? true)
        + `<div id="key-admin-confirm" hidden>${checkbox('confirm_admin', '确认授予管理员 API 权限')}</div>`,
      onMount: form => {
        const roleChanged = () => { const elevated = form.elements.role.value === 'admin' && key?.role !== 'admin'; form.querySelector('#key-admin-confirm').hidden = !elevated; form.elements.confirm_admin.required = elevated; };
        form.elements.role.addEventListener('change', roleChanged); roleChanged();
        form.querySelector('#key-load-models').addEventListener('click', () => {
          const models = groupModels(channels, parseGroups(form.elements.groups.value));
          if (!models.length) { toast('所选分组暂无模型，请先给渠道设置相同分组'); return; }
          form.elements.models.value = models.map(model => `<${model}>`).join('\n');
        });
        const chipsRoot = form.querySelector('[data-key-group-chips]');
        const newGroupInput = form.querySelector('[data-key-new-group]');
        const renderChips = () => {
          const active = parseGroups(form.elements.groups.value);
          chipsRoot.innerHTML = knownGroups.map(value => `<button class="group-chip" type="button" data-add-group="${esc(value)}" aria-pressed="${active.includes(value)}">${esc(value)}</button>`).join('');
        };
        renderChips();
        chipsRoot.addEventListener('click', event => {
          const button = event.target.closest('[data-add-group]');
          if (!button) return;
          const values = parseGroups(form.elements.groups.value);
          const selected = button.dataset.addGroup;
          const next = values.includes(selected) ? values.filter(value => value !== selected) : [...values, selected];
          form.elements.groups.value = (next.length ? next : ['default']).join(', ');
          renderChips();
        });
        const addCustomGroup = async () => {
          const added = parseGroups(newGroupInput.value);
          if (!added.length) return;
          const current = parseGroups(form.elements.groups.value);
          for (const item of added) {
            if (!knownGroups.includes(item)) knownGroups.push(item);
            if (!current.includes(item)) current.push(item);
          }
          knownGroups.sort();
          form.elements.groups.value = current.join(', ');
          newGroupInput.value = '';
          renderChips();
          try {
            const saved = await api.patch('/api-keys', { revision: data.revision, group_action: 'add', group_name: added.join(', ') });
            if (saved?.revision) data.revision = saved.revision;
          } catch {}
        };
        newGroupInput.addEventListener('keydown', event => {
          if ((event.key === 'Enter' || event.key === ',') && !event.isComposing) {
            event.preventDefault();
            addCustomGroup();
          }
        });
        newGroupInput.addEventListener('blur', () => { if (newGroupInput.value.trim()) addCustomGroup(); });
      },
      onSubmit: async form => {
        const groups = parseGroups(form.elements.groups.value);
        if (!groups.length) throw new Error('请至少填写一个分组');
        const role = form.elements.role.value;
        if (role === 'admin' && key?.role !== 'admin' && !form.elements.confirm_admin.checked) throw new Error('请确认管理员权限');
        const body = { revision: data.revision, ...(key ? { id: key.id } : {}), name: form.elements.name.value.trim(), groups, role, enabled: form.elements.enabled.checked };
        const modelsText = form.elements.models.value.trim();
        if (!key || modelsText !== originalModels) body.models = [...new Set(modelsText.split(/\r?\n/).map(value => value.trim()).filter(Boolean))];
        if (!key || form.elements.rate_limit.value.trim() !== originalRate) body.rate_limit = form.elements.rate_limit.value.trim();
        const result = key ? await api.patch('/api-keys', body) : await api.post('/api-keys', body);
        if (key) { await load(); toast('密钥已保存'); }
        else {
          const secret = result.api_key; delete result.api_key;
          try { await load(); toast('密钥已创建'); }
          catch (error) { toast(`密钥已创建，列表刷新失败：${error.message}`); }
          setTimeout(() => showSecret(secret), 0);
        }
      },
    });
  }
  function createGroupDialog() {
    dialog({ title: '新建 Key 分组', submitLabel: '保存分组',
      body: field('new_group_name', '分组名称（可输入多个，用逗号分隔）', '', { required: true, placeholder: '例如：vip, codex专属, claude-pro', help: '分组用于隔离 API Key 与渠道的访问关系；创建后可在 API Key 和渠道中直接选用。' }),
      onSubmit: async form => {
        const names = parseGroups(form.elements.new_group_name.value);
        if (!names.length) throw new Error('请输入分组名称');
        await api.patch('/api-keys', { revision: data.revision, group_action: 'add', group_name: names.join(',') });
        group = names[0];
        await load();
        toast(`分组「${names.join(', ')}」已添加`);
      },
    });
  }
  root.addEventListener('click', async event => {
    const tab = event.target.closest('[data-key-group]');
    if (tab) { group = tab.dataset.keyGroup; draw(); return; }
    const button = event.target.closest('[data-key-action]');
    if (!button || button.disabled) return;
    const action = button.dataset.keyAction, key = data.api_keys.find(item => item.id === button.dataset.keyId);
    if (action === 'create') return edit();
    if (action === 'manage-groups' || action === 'create-group') return openGroupManager({ onChanged: load });
    if (action === 'quick-add-group') {
      const inputEl = root.querySelector('#quick-group-input');
      const names = parseGroups(inputEl?.value || '');
      if (!names.length) { inputEl?.focus(); toast('请先在输入框填写新分组名称'); return; }
      button.disabled = true;
      try {
        await api.patch('/api-keys', { revision: data.revision, group_action: 'add', group_name: names.join(', ') });
        group = names[0];
        await load();
        toast(`已添加分组：${names.join(', ')}`);
      } catch (err) {
        toast(err.message);
        button.disabled = false;
      }
      return;
    }
    if (action === 'remove-group') {
      const name = button.dataset.groupName;
      await api.patch('/api-keys', { revision: data.revision, group_action: 'remove', group_name: name });
      if (group === name) group = '';
      await load();
      toast(`已移除自定义分组「${name}」`);
      return;
    }
    if (action === 'edit') return edit(key);
    if (action === 'refresh') { button.disabled = true; try { await load(); } catch (error) { toast(error.message); button.disabled = false; } return; }
    if (action === 'toggle' || action === 'revoke') {
      confirmAction({ title: action === 'revoke' ? '吊销密钥' : key.enabled ? '停用密钥' : '启用密钥', message: action === 'revoke' ? `吊销“${key.name || key.id}”？此密钥将无法恢复。` : `${key.enabled ? '停用' : '启用'}“${key.name || key.id}”？配置与已有使用记录会保留。`, label: '确认', action: async () => {
        if (action === 'revoke') await api.remove('/api-keys', { revision: data.revision, id: key.id });
        else await api.patch('/api-keys', { revision: data.revision, id: key.id, enabled: !key.enabled });
        await load(); toast('密钥已更新');
      } });
    }
  }, { signal });
  await load();
}
