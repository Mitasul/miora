import { mountIcons, icon } from './icons.js';
import { request } from './api.js';
mountIcons();
const form = document.querySelector('#login-form');
const submit = document.querySelector('#login-submit');
const errorBox = document.querySelector('#login-error');
const password = document.querySelector('#password');
const toggle = document.querySelector('#toggle-password');
let submitting = false;
function checkExistingSession() {
  request('/session', { redirect: false }).then(session => {
    if (session?.user) window.location.replace('/admin#overview');
  }).catch(() => {});
}
checkExistingSession();
window.addEventListener('pageshow', event => { if (event.persisted) checkExistingSession(); });
toggle.addEventListener('click', () => {
  const visible = password.type === 'password';
  password.type = visible ? 'text' : 'password';
  toggle.setAttribute('aria-pressed', String(visible));
  toggle.setAttribute('aria-label', visible ? '隐藏密码' : '显示密码');
  toggle.innerHTML = icon(visible ? 'eyeOff' : 'eye');
});
form.addEventListener('submit', async event => {
  event.preventDefault();
  if (submitting || !form.reportValidity()) return;
  const username = form.elements.username.value.trim();
  if (!username) { form.elements.username.focus(); return; }
  submitting = true; submit.disabled = true; form.setAttribute('aria-busy', 'true'); errorBox.hidden = true;
  submit.innerHTML = '<span class="spinner small"></span><span>正在验证…</span>';
  try {
    await request('/login', { method: 'POST', body: { username, password: password.value }, redirect: false });
    password.value = '';
    window.location.replace('/admin#overview');
  } catch (error) { errorBox.textContent = error.message; errorBox.hidden = false; password.focus(); }
  finally { submitting = false; submit.disabled = false; form.removeAttribute('aria-busy'); submit.innerHTML = `<span>登录控制台</span>${icon('arrow')}`; }
});
