import * as api from './api.js';
import { esc, select, field, failure } from './ui.js';

export function playgroundDelta(event) {
  if (event.error || ['error', 'response.failed'].includes(event.type)) throw new Error(event.error?.message || event.response?.error?.message || '上游流式请求失败');
  if (event.type === 'response.output_text.delta') return { text: event.delta || '' };
  if (event.type === 'response.reasoning_text.delta' || event.type === 'response.reasoning_summary_text.delta') return { thinking: event.delta || '' };
  if (event.type === 'response.function_call_arguments.delta') return { tool: event.delta || '' };
  if (event.type === 'response.completed' || event.type === 'response.incomplete') return { usage: event.response?.usage, terminal: true };
  const choice = event.choices?.[0], delta = choice?.delta;
  return { text: delta?.content || '', thinking: delta?.reasoning_content || '', tool: delta?.tool_calls ? JSON.stringify(delta.tool_calls) + '\n' : '', usage: event.usage, terminal: Boolean(choice?.finish_reason) };
}

export async function mountPlayground(root, { signal }) {
  const data = await api.get('/api-keys', { signal });
  if (signal.aborted) return;
  const keys = data.api_keys.filter(key => key.enabled);
  if (!keys.length) { root.innerHTML = '<section class="card"><p>先添加并启用一个 API Key。</p></section>'; return; }
  root.innerHTML = `<div class="page-heading"><h1>对话调试台</h1><a class="button" href="#testing">返回拨测</a></div><section class="card playground-controls"><form id="playground-form"><div class="toolbar">${select('position', '使用密钥', keys.map(key => ({ value: key.position, label: `${key.name || `Key ${key.position}`} · ${key.groups.join(', ')}` })), keys[0].position)}${select('endpoint', '协议', [{ value: '/v1/chat/completions', label: 'Chat Completions' }, { value: '/v1/responses', label: 'Responses' }])}${field('limit', '输出 Token 上限（可选）', '', { type: 'number', min: 1, step: 1 })}</div><div class="playground-models">${[0, 1, 2].map(index => select(`model${index}`, index ? `对比模型 ${index + 1}（可选）` : '主模型', [{ value: '', label: '加载中' }])).join('')}</div>${field('prompt', '消息', '', { type: 'textarea', rows: 3, required: true, placeholder: '输入消息；每个模型保留独立的多轮上下文' })}<div class="page-actions"><button type="submit" class="button primary">发送</button><button type="button" class="button" data-playground-stop hidden>停止</button><button type="button" class="button" data-playground-clear>清空对话</button><span class="hint">真实调用，正常计费</span></div><p role="alert" data-playground-error class="text-danger"></p></form></section><div class="playground-columns">${[0,1,2].map(index => `<section class="card playground-lane" data-lane="${index}" ${index ? 'hidden' : ''}><h2 data-lane-title>模型</h2><div class="playground-messages" data-lane-messages></div><p data-lane-status class="hint" role="status"></p></section>`).join('')}</div>`;
  const form = root.querySelector('#playground-form'), error = root.querySelector('[data-playground-error]');
  const histories = [[], [], []]; let running = false, requestController, loadController, loaded = 0;
  signal.addEventListener('abort', () => { requestController?.abort(); loadController?.abort(); }, { once: true });
  const lane = index => root.querySelector(`[data-lane="${index}"]`);
  function clearLane(index) { histories[index] = []; lane(index).querySelector('[data-lane-messages]').textContent = ''; lane(index).querySelector('[data-lane-status]').textContent = ''; }
  function syncLanes() {
    for (let index = 0; index < 3; index++) {
      const model = form.elements[`model${index}`].value; lane(index).hidden = !model; lane(index).querySelector('[data-lane-title]').textContent = model || '模型';
    }
  }
  async function loadModels() {
    loadController?.abort(); loadController = new AbortController(); const generation = ++loaded;
    form.querySelector('[type="submit"]').disabled = true;
    try {
      const catalog = await api.get(`/test/models?${new URLSearchParams({ position: form.elements.position.value, endpoint: form.elements.endpoint.value })}`, { signal: loadController.signal });
      if (signal.aborted || generation !== loaded) return;
      const models = [...new Set(catalog.models.map(row => row.model))].sort();
      for (let index = 0; index < 3; index++) {
        clearLane(index); form.elements[`model${index}`].innerHTML = `${index ? '<option value="">不对比</option>' : ''}${models.map(model => `<option value="${esc(model)}">${esc(model)}</option>`).join('')}`;
      }
      if (!models.length) throw new Error('此密钥暂无可用模型');
      form.querySelector('[type="submit"]').disabled = false; error.textContent = ''; syncLanes();
    } catch (problem) { if (!signal.aborted && generation === loaded) error.textContent = problem.message; }
  }
  form.elements.position.addEventListener('change', loadModels); form.elements.endpoint.addEventListener('change', loadModels);
  for (let index = 0; index < 3; index++) form.elements[`model${index}`].addEventListener('change', () => { clearLane(index); syncLanes(); });
  form.querySelector('[data-playground-stop]').addEventListener('click', () => requestController?.abort());
  form.querySelector('[data-playground-clear]').addEventListener('click', () => { if (!running) for (let index = 0; index < 3; index++) clearLane(index); });
  form.addEventListener('submit', async event => {
    event.preventDefault(); if (running) return;
    const prompt = form.elements.prompt.value.trim(); if (!prompt) return;
    const selected = [0,1,2].filter(index => form.elements[`model${index}`].value); if (!selected.length) return;
    const position = Number(form.elements.position.value), endpoint = form.elements.endpoint.value, limit = form.elements.limit.value;
    running = true; requestController = new AbortController(); error.textContent = '';
    const controls = [...form.querySelectorAll('input,select,textarea,button')]; controls.forEach(control => { control.disabled = true; });
    const stop = form.querySelector('[data-playground-stop]'); stop.hidden = false; stop.disabled = false;
    await Promise.all(selected.map(async index => {
      const panel = lane(index), output = panel.querySelector('[data-lane-messages]'), status = panel.querySelector('[data-lane-status]');
      const question = document.createElement('div'); question.className = 'playground-user'; question.textContent = prompt; output.append(question);
      const response = document.createElement('div'); response.className = 'playground-assistant'; response.innerHTML = '<details hidden><summary>思考过程</summary><pre data-thinking></pre></details><pre data-answer></pre><details hidden><summary>工具调用（仅展示，不执行）</summary><pre data-tool></pre></details>'; output.append(response);
      let text = '', thinking = '', tool = '', usage, terminal = false, scheduled = 0; const started = performance.now();
      const draw = () => {
        scheduled = 0; response.querySelector('[data-answer]').textContent = text;
        const thought = response.querySelector('[data-thinking]'); thought.textContent = thinking; thought.closest('details').hidden = !thinking;
        const calls = response.querySelector('[data-tool]'); calls.textContent = tool; calls.closest('details').hidden = !tool;
      };
      const model = form.elements[`model${index}`].value, turns = [...histories[index], { role: 'user', content: prompt }];
      const request = endpoint === '/v1/responses' ? { model, stream: true, input: turns } : { model, stream: true, messages: turns };
      if (limit) request[endpoint === '/v1/responses' ? 'max_output_tokens' : 'max_tokens'] = Number(limit);
      status.textContent = '等待上游…';
      try {
        const meta = await api.stream('/test/stream', { position, endpoint, request }, { signal: requestController.signal, onEvent: event => {
          const delta = playgroundDelta(event); text += delta.text || ''; thinking += delta.thinking || ''; tool += delta.tool || ''; if (delta.usage) usage = delta.usage; terminal ||= delta.terminal;
          if (!scheduled) scheduled = requestAnimationFrame(draw); status.textContent = '输出中…';
        } });
        if (!terminal) throw new Error('连接结束，但没有收到完成事件');
        if (!tool) histories[index] = [...turns, { role: 'assistant', content: text, ...(thinking ? { reasoning_content: thinking } : {}) }];
        const tokens = usage ? ` · 输入 ${usage.prompt_tokens ?? usage.input_tokens ?? '—'} / 输出 ${usage.completion_tokens ?? usage.output_tokens ?? '—'} · 缓存 ${usage.prompt_tokens_details?.cached_tokens ?? usage.input_tokens_details?.cached_tokens ?? '—'}` : ' · 用量未报告';
        status.textContent = `${((performance.now() - started) / 1000).toFixed(2)} 秒${tokens}${tool ? ' · 工具轮次未写入聊天上下文' : ''}${meta.requestId ? ` · ${meta.requestId}` : ''}`;
      } catch (problem) { status.textContent = requestController.signal.aborted ? '已停止 · 本轮未写入上下文' : `失败：${problem.message}`; }
      finally { if (scheduled) cancelAnimationFrame(scheduled); draw(); }
    }));
    running = false; controls.forEach(control => { control.disabled = false; }); stop.hidden = true;
    if (!requestController.signal.aborted) form.elements.prompt.value = '';
  });
  await loadModels();
}
