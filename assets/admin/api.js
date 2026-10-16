const BASE = '/admin/api';
let csrfToken = '';
export class ApiError extends Error {
  constructor(message, status = 0, code = 'network_error') { super(message); this.name = 'ApiError'; this.status = status; this.code = code; }
}
export function setSession(session) { csrfToken = session?.csrf_token || ''; }
export async function request(path, { method = 'GET', body, signal, redirect = true, timeoutMs = 25000 } = {}) {
  const write = !['GET', 'HEAD'].includes(method.toUpperCase());
  const multipart = body instanceof FormData;
  const headers = { Accept: 'application/json' };
  if (body !== undefined && !multipart) headers['Content-Type'] = 'application/json';
  if (write && csrfToken) headers['X-CSRF-Token'] = csrfToken;
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  const abort = () => controller.abort();
  if (signal?.aborted) controller.abort();
  else signal?.addEventListener('abort', abort, { once: true });
  try {
    const response = await fetch(`${BASE}${path}`, { method, headers, body: body === undefined ? undefined : multipart ? body : JSON.stringify(body), credentials: 'same-origin', cache: 'no-store', signal: controller.signal });
    let data = null;
    const text = await response.text();
    if (text) { try { data = JSON.parse(text); } catch { throw new ApiError('服务返回了无法解析的响应，请检查服务状态。', response.status, 'invalid_response'); } }
    if (!response.ok) {
      if (response.status === 401 && redirect) { setSession(null); window.location.replace('/login'); }
      const message = data?.error?.message || `请求失败（HTTP ${response.status}）`;
      throw new ApiError(response.status === 409 ? `${message}。配置已变更，请关闭表单并刷新后重试。` : message, response.status, data?.error?.code);
    }
    return data;
  } catch (error) {
    if (error instanceof ApiError) throw error;
    if (signal?.aborted) throw error;
    if (error.name === 'AbortError') throw new ApiError('请求超时，请检查连接后重试。');
    throw new ApiError('无法连接到管理服务，请检查网络后重试。');
  } finally { clearTimeout(timeout); signal?.removeEventListener('abort', abort); }
}
export async function session() { const data = await request('/session'); setSession(data); return data; }
export function parseSseFrames(text) {
  const frames = []; let match;
  while ((match = /\r?\n\r?\n/.exec(text))) {
    frames.push(text.slice(0, match.index)); text = text.slice(match.index + match[0].length);
  }
  return { frames, rest: text };
}
export async function stream(path, body, { signal, onEvent }) {
  const response = await fetch(`${BASE}${path}`, { method: 'POST', credentials: 'same-origin', cache: 'no-store', signal, headers: { 'Content-Type': 'application/json', Accept: 'text/event-stream', 'X-CSRF-Token': csrfToken }, body: JSON.stringify(body) });
  if (!response.ok) {
    const text = await response.text(); let data; try { data = JSON.parse(text); } catch {}
    if (response.status === 401) { setSession(null); window.location.replace('/login'); }
    throw new ApiError(data?.error?.message || `请求失败（HTTP ${response.status}）`, response.status);
  }
  const reader = response.body.getReader(), decoder = new TextDecoder(); let buffer = '';
  const consume = frame => {
    const data = frame.split(/\r?\n/).filter(line => line.startsWith('data:')).map(line => line.slice(5).trimStart()).join('\n');
    if (!data || data === '[DONE]') return;
    onEvent(JSON.parse(data));
  };
  try {
    while (true) {
      const { value, done } = await reader.read(); buffer += decoder.decode(value, { stream: !done });
      const parsed = parseSseFrames(buffer); buffer = parsed.rest;
      for (const frame of parsed.frames) consume(frame);
      if (buffer.length > 1024 * 1024) throw new ApiError('SSE 数据帧超过 1 MiB');
      if (done) { if (buffer.trim()) consume(buffer); break; }
    }
    return { requestId: response.headers.get('x-request-id') || '' };
  } finally { await reader.cancel(); reader.releaseLock(); }
}
export const get = (path, options) => request(path, options);
export const post = (path, body = {}) => request(path, { method: 'POST', body });
export const patch = (path, body) => request(path, { method: 'PATCH', body });
export const put = (path, body) => request(path, { method: 'PUT', body });
export const remove = (path, body) => request(path, { method: 'DELETE', body });
export function upload(path, file, options = {}) {
  const body = new FormData();
  body.append('file', file);
  body.append('options', JSON.stringify(options));
  return request(path, { method: 'POST', body });
}
