import * as api from './api.js';
import { dialog, esc, toast } from './ui.js';

export function virtualModelDocument(rows) {
  const definitions = {};
  for (const row of rows) {
    const name = row.name.trim();
    if (!name || Object.hasOwn(definitions, name)) throw new Error('虚拟模型名称不能为空或重复');
    if (!row.targets.length) throw new Error(`${name} 至少需要一个目标`);
    const targets = row.targets.map(target => {
      const result = { ...(target.source || {}) };
      delete result.virtual; delete result.provider; delete result.model; delete result.on_status;
      const model = target.model.trim(); if (!model) throw new Error(`${name} 的目标模型不能为空`);
      if (target.provider === '@virtual') result.virtual = model;
      else { if (!target.provider) throw new Error('请选择渠道'); result.provider = target.provider; result.model = model; }
      if (target.statuses.trim()) {
        const statuses = target.statuses.split(/[,，]/).map(value => Number(value.trim()));
        if (statuses.some(status => !Number.isInteger(status) || status < 400 || status > 599)) throw new Error('回落状态填写 400–599，用逗号分隔');
        result.on_status = statuses;
      }
      return result;
    });
    Object.defineProperty(definitions, name, { value: targets, enumerable: true });
  }
  return definitions;
}

export async function openVirtualModels(onSaved) {
  const data = await api.get('/virtual-models');
  const rows = Object.entries(data.virtual_models).map(([name, targets]) => ({ name, targets: targets.map(source => ({ source, provider: source.virtual ? '@virtual' : source.provider, model: source.virtual || source.model, statuses: (source.on_status || []).join(', ') })) }));
  let form;
  const freshTarget = () => ({ provider: data.providers[0]?.provider || '', model: '', statuses: '' });
  function capture() {
    for (const group of form.querySelectorAll('[data-virtual-row]')) {
      const row = rows[Number(group.dataset.virtualRow)]; row.name = group.querySelector('[data-virtual-name]').value;
      for (const item of group.querySelectorAll('[data-virtual-target]')) {
        const target = row.targets[Number(item.dataset.virtualTarget)];
        target.provider = item.querySelector('[data-target-provider]').value; target.model = item.querySelector('[data-target-model]').value; target.statuses = item.querySelector('[data-target-status]').value;
      }
    }
  }
  function draw() {
    form.querySelector('[data-virtual-list]').innerHTML = rows.map((row, index) => `<fieldset class="virtual-model-group" data-virtual-row="${index}"><div class="toolbar"><label class="field"><span>对外模型名</span><input data-virtual-name required value="${esc(row.name)}"></label><button class="button small text-danger" type="button" data-remove-virtual="${index}">删除模型</button></div>${row.targets.map((target, position) => `<div class="virtual-model-target" data-virtual-target="${position}"><span class="hint">${position + 1}</span><label class="field"><span>渠道 / 嵌套模型</span><select data-target-provider><option value="@virtual" ${target.provider === '@virtual' ? 'selected' : ''}>引用另一个虚拟模型</option>${data.providers.map(provider => `<option value="${esc(provider.provider)}" ${target.provider === provider.provider ? 'selected' : ''}>${esc(provider.provider)}${provider.enabled ? '' : ' · 已停用'}</option>`).join('')}</select></label><label class="field"><span>目标模型（配置名）</span><input data-target-model required value="${esc(target.model)}" list="virtual-model-options"></label><label class="field"><span>仅这些状态时回落（可选）</span><input data-target-status value="${esc(target.statuses)}" placeholder="429, 502, 503"></label><button class="button small text-danger" type="button" data-remove-target="${position}" data-owner="${index}">移除</button></div>`).join('')}<button class="button small" type="button" data-add-target="${index}">添加回落目标</button></fieldset>`).join('');
    form.querySelector('[data-virtual-options]').innerHTML = [...new Set([...rows.map(row => row.name), ...data.providers.flatMap(provider => provider.models)])].map(model => `<option value="${esc(model)}">`).join('');
  }
  form = dialog({ title: '虚拟模型', size: 'wide', submitLabel: '保存', body: '<p class="hint">按顺序尝试，首个目标不设状态条件；嵌套引用在保存时展开并检查循环。目标渠道仍受 API Key 分组限制。</p><div data-virtual-list></div><datalist id="virtual-model-options" data-virtual-options></datalist><button type="button" class="button" data-add-virtual>添加虚拟模型</button>',
    onMount: mounted => {
      form = mounted; draw(); form.addEventListener('click', event => {
        const target = event.target.closest('button'); if (!target) return;
        if (!['addVirtual','removeVirtual','addTarget','removeTarget'].some(name => Object.hasOwn(target.dataset, name))) return;
        capture();
        if (Object.hasOwn(target.dataset, 'addVirtual')) rows.push({ name: '', targets: [freshTarget()] });
        if (Object.hasOwn(target.dataset, 'removeVirtual')) rows.splice(Number(target.dataset.removeVirtual), 1);
        if (Object.hasOwn(target.dataset, 'addTarget')) rows[Number(target.dataset.addTarget)].targets.push(freshTarget());
        if (Object.hasOwn(target.dataset, 'removeTarget')) rows[Number(target.dataset.owner)].targets.splice(Number(target.dataset.removeTarget), 1);
        draw();
      });
    }, onSubmit: async () => { capture(); await api.put('/virtual-models', { revision: data.revision, virtual_models: virtualModelDocument(rows) }); toast('虚拟模型已保存'); await onSaved(); }
  });
}
