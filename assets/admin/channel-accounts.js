import * as api from './api.js';
import { icon } from './icons.js';
import { badge, dialog, esc, toast } from './ui.js';

export function formatAccountIdentity(key) {
  const oauth = key.oauth || {};
  const primary = oauth.email || oauth.account_id || key.credential || `账号 #${key.position}`;
  const plan = oauth.plan_type ? badge(String(oauth.plan_type).toUpperCase(), 'purple') : '';
  const secondary = oauth.email && oauth.account_id ? oauth.account_id : oauth.email ? key.credential : `凭据 #${key.position}`;
  return `<strong>${esc(primary)}</strong> ${plan}<span class="subline">${esc(secondary)}</span>`;
}

export function formatOAuthState(key) {
  const oauth = key.oauth || {};
  if (oauth.refreshable) {
    return oauth.expires_at
      ? `${badge('OAuth 账号', 'green')}<span class="subline">到期 ${esc(new Date(oauth.expires_at * 1000).toLocaleString())}</span>`
      : `${badge('OAuth 账号', 'purple')}<span class="subline">点击「刷新授权」同步邮箱与到期时间</span>`;
  }
  if (oauth.email || oauth.plan_type || oauth.expires_at) {
    return `${badge('Token 凭据', 'purple')}${oauth.expires_at ? `<span class="subline">到期 ${esc(new Date(oauth.expires_at * 1000).toLocaleString())}</span>` : ''}`;
  }
  return badge('API Key / Token');
}

export async function openChannelAccounts(provider) {
  let data = await api.get(`/channels/keys?${new URLSearchParams({ provider })}`);
  let form, timer, pollTimer, busy = false, closed = false, oauthSession = null;
  const controller = new AbortController();
  const isOAuthEngine = ['codex', 'claude', 'antigravity'].includes(data.engine);
  const testState = new Map();
  function draw() {
    const view = form.querySelector('[data-account-list]');
    view.innerHTML = data.keys.length ? `<div class="table-scroll account-pool-scroll"><table class="account-pool-table"><thead><tr><th>账号 / 邮箱</th><th>状态</th><th>冷却</th><th>失败</th><th>认证</th><th>操作</th></tr></thead><tbody>${data.keys.map(key => {
      const st = testState.get(key.position);
      const testBadge = st?.status === 'testing' ? badge('正在发 hi…', 'purple') : st?.status === 'ok' ? badge(`${st.elapsed_ms} ms 正常`, 'green') : st?.status === 'error' ? badge(`失败 (${st.http_status || 'ERR'})`, 'orange') : '';
      return `<tr class="account-pool-row"><td class="account-col-id">${formatAccountIdentity(key)}${testBadge ? `<div>${testBadge}</div>` : ''}</td><td class="account-col-status">${badge(key.enabled ? '启用中' : '已停用', key.enabled ? 'green' : 'orange')}</td><td class="account-col-cooldown">${key.cooldown_seconds ? badge(`冷却 ${key.cooldown_seconds}s`, 'orange') : '正常'}</td><td class="account-col-fail">${key.failure_count}</td><td class="account-col-auth">${formatOAuthState(key)}</td><td class="account-col-action"><div class="table-actions"><button type="button" class="button small" data-key-test="${key.position}">${icon('pulse')}测活</button><button type="button" class="button small" data-key-toggle="${key.position}">${key.enabled ? '停用' : '启用'}</button>${key.oauth?.refreshable ? `<button type="button" class="button small" data-key-refresh="${key.position}">${icon('refresh')}刷新授权</button>` : ''}<button type="button" class="button small text-danger" data-key-remove="${key.position}">${icon('trash')}移除</button></div></td></tr>`;
    }).join('')}</tbody></table></div>` : `<p class="hint">暂无账号或凭据。</p>`;
  }
  async function reload() {
    data = await api.get(`/channels/keys?${new URLSearchParams({ provider })}`, { signal: controller.signal });
    if (!closed) draw();
  }
  async function mutate(body, method = 'PATCH', message = '已保存') {
    if (busy) return false;
    busy = true; form.setAttribute('aria-busy', 'true');
    const error = form.querySelector('[data-account-error]'); error.textContent = '';
    try {
      await api.request('/channels/keys', { method, body: { provider, revision: data.revision, ...body }, signal: controller.signal, timeoutMs: 60000 });
      await reload(); toast(message);
      return true;
    } catch (failure) {
      if (!closed) error.textContent = failure.message;
      return false;
    } finally {
      busy = false; form.removeAttribute('aria-busy');
    }
  }
  const oauthSection = isOAuthEngine
    ? `<section class="form-section"><div class="form-section-header"><h2>OAuth 授权添加账号</h2></div><div class="toolbar"><button type="button" class="button primary" data-oauth-start>${icon('key')}打开浏览器 OAuth 授权</button></div><div data-oauth-flow hidden><label class="field"><span>授权链接</span><input data-oauth-url type="url" readonly></label><label class="field"><span>${data.engine === 'claude' ? '粘贴授权码 (code#state)' : '本机自动捕获回调，也可粘贴回调 URL'}</span><input data-oauth-callback type="text" placeholder="${data.engine === 'claude' ? '粘贴 code#state' : '粘贴回调 URL 或授权码'}" autocomplete="off" spellcheck="false"></label><button type="button" class="button primary" data-oauth-exchange>${icon('check')}验证并绑定</button></div></section>`
    : '';
  form = dialog({ title: `${provider} · 账号管理`, size: 'wide', submitLabel: '关闭', showCancel: false,
    body: `<div data-account-list></div>${oauthSection}<section class="form-section"><div class="form-section-header"><h2>${isOAuthEngine ? '导入文件或批量添加' : '添加凭据'}</h2></div><label class="field"><span>上传 JSON 文件</span><input data-account-file type="file" accept=".json,application/json"></label><label class="field"><span>粘贴 JSON 或每行一个凭据</span><textarea data-account-import rows="3" placeholder="粘贴凭据 JSON 或每行一个 Key / Token" autocomplete="off" spellcheck="false"></textarea></label><button type="button" class="button primary" data-account-upload>${icon('plus')}导入保存</button></section><p data-account-error class="text-danger" role="alert"></p>`,
    onSubmit: async () => {}, onClose: () => { closed = true; clearInterval(timer); clearInterval(pollTimer); controller.abort(); },
    onMount: mounted => {
      form = mounted; draw();
      form.addEventListener('click', async event => {
        const testBtn = event.target.closest('[data-key-test]');
        const toggle = event.target.closest('[data-key-toggle]');
        const refresh = event.target.closest('[data-key-refresh]');
        const remove = event.target.closest('[data-key-remove]');
        if (testBtn) {
          const position = Number(testBtn.dataset.keyTest);
          testState.set(position, { status: 'testing' });
          draw();
          try {
            const res = await api.request('/test', { method: 'POST', signal: controller.signal, timeoutMs: 65000, body: { direct: true, provider, key_position: position } });
            if (res.ok) {
              testState.set(position, { status: 'ok', elapsed_ms: res.elapsed_ms, http_status: res.http_status });
              toast(`账号 #${position} 测活成功（${res.elapsed_ms} ms）`);
            } else {
              const msg = res.response?.error?.message || res.response?.message || `HTTP ${res.http_status}`;
              testState.set(position, { status: 'error', http_status: res.http_status });
              form.querySelector('[data-account-error]').textContent = `账号 #${position} 测活失败：${msg}`;
            }
          } catch (err) {
            testState.set(position, { status: 'error', http_status: 'ERR' });
            form.querySelector('[data-account-error]').textContent = `账号 #${position} 测活失败：${err.message}`;
          }
          if (!closed) draw();
          return;
        }
        if (toggle) {
          const position = Number(toggle.dataset.keyToggle);
          const key = data.keys.find(item => item.position === position);
          await mutate({ position, enabled: !key.enabled });
        }
        if (refresh) await mutate({ position: Number(refresh.dataset.keyRefresh), refresh: true }, 'PATCH', '已刷新 OAuth 授权并同步账号信息');
        if (remove) await mutate({ position: Number(remove.dataset.keyRemove), remove: true }, 'PATCH', '账号已移除');
        if (event.target.closest('[data-oauth-start]')) {
          try {
            clearInterval(pollTimer);
            oauthSession = await api.get(`/channels/keys?${new URLSearchParams({ oauth: 'authorize', provider: data.engine })}`, { signal: controller.signal });
            const flow = form.querySelector('[data-oauth-flow]');
            const urlInput = form.querySelector('[data-oauth-url]');
            urlInput.value = oauthSession.authorize_url;
            flow.hidden = false;
            window.open(oauthSession.authorize_url, '_blank', 'noopener');
            form.querySelector('[data-oauth-callback]').focus();
            if (oauthSession.auto_callback && oauthSession.state) {
              pollTimer = setInterval(async () => {
                if (closed || busy || !oauthSession?.state) return;
                try {
                  const st = await api.get(`/channels/keys?${new URLSearchParams({ oauth: 'status', state: oauthSession.state })}`, { signal: controller.signal });
                  if (st.status === 'completed' && st.credential) {
                    clearInterval(pollTimer);
                    const ok = await mutate({ import: [st.credential] }, 'POST', `OAuth 自动授权成功${st.oauth?.email ? `（${st.oauth.email}）` : ''}`);
                    if (ok && !closed) form.querySelector('[data-oauth-flow]').hidden = true;
                  }
                } catch {}
              }, 1500);
            }
          } catch (failure) {
            form.querySelector('[data-account-error]').textContent = failure.message;
          }
        }
        if (event.target.closest('[data-oauth-exchange]')) {
          const code = form.querySelector('[data-oauth-callback]').value.trim();
          if (!code) { form.querySelector('[data-account-error]').textContent = '请先粘贴回调 URL 或授权码'; return; }
          clearInterval(pollTimer);
          const ok = await mutate({
            oauth_exchange: {
              provider: data.engine,
              code,
              state: oauthSession?.state || undefined,
              code_verifier: oauthSession?.code_verifier || '',
              redirect_uri: oauthSession?.redirect_uri || undefined,
            },
          }, 'POST', 'OAuth 登录成功，账号已绑定');
          if (ok) {
            form.querySelector('[data-oauth-callback]').value = '';
            form.querySelector('[data-oauth-flow]').hidden = true;
          }
        }
        if (event.target.closest('[data-account-upload]')) {
          try {
            const file = form.querySelector('[data-account-file]').files[0];
            if (file && file.size > 128 * 1024) throw new Error('凭据文件不能超过 128 KiB');
            const text = file ? await file.text() : form.querySelector('[data-account-import]').value.trim();
            if (!text) throw new Error('请先选择账号 JSON 文件或粘贴凭据内容');
            const imported = /^[\[{]/.test(text) ? JSON.parse(text) : text.split(/\r?\n/).map(value => value.trim()).filter(Boolean);
            const ok = await mutate({ import: imported }, 'POST', '账号凭据已导入');
            if (ok) {
              form.querySelector('[data-account-import]').value = '';
              form.querySelector('[data-account-file]').value = '';
            }
          } catch (failure) { form.querySelector('[data-account-error]').textContent = failure.message; }
        }
      });
      timer = setInterval(async () => {
        if (busy || closed || document.hidden) return;
        busy = true;
        try { await reload(); } catch (failure) { if (!closed) form.querySelector('[data-account-error]').textContent = failure.message; }
        finally { busy = false; }
      }, 3000);
    }
  });
}
