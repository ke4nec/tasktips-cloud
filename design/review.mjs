// Visual review, interaction checks, and screenshot export for the offline design.
// Start an isolated Chromium with --remote-debugging-port=9236, then run this file.
import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const root = new URL('./', import.meta.url);
const base = new URL('index.html', root).href;
const port = process.env.DESIGN_CDP_PORT || '9236';
const targets = await (
  await fetch(`http://127.0.0.1:${port}/json/list`)
).json();
const target = targets.find((entry) => entry.type === 'page');
assert(target, 'Open a blank page in the dedicated review browser.');
const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve) =>
  socket.addEventListener('open', resolve, { once: true }),
);
let sequence = 0;
const pending = new Map();
const errors = [];
const network = [];
const layoutChecks = [];
const interactionChecks = [];
socket.addEventListener('message', ({ data }) => {
  const message = JSON.parse(data);
  if (message.id) {
    const call = pending.get(message.id);
    if (!call) return;
    pending.delete(message.id);
    clearTimeout(call.timer);
    if (message.error) call.reject(new Error(JSON.stringify(message.error)));
    else call.resolve(message.result);
  } else if (message.method === 'Runtime.exceptionThrown')
    errors.push(message.params.exceptionDetails);
  else if (
    message.method === 'Runtime.consoleAPICalled' &&
    message.params.type === 'error'
  )
    errors.push(message.params.args);
  else if (message.method === 'Network.requestWillBeSent')
    network.push(message.params.request.url);
});
function call(method, params = {}) {
  return new Promise((resolve, reject) => {
    const id = ++sequence;
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`Timeout: ${method}`));
    }, 10000);
    pending.set(id, { resolve, reject, timer });
    socket.send(JSON.stringify({ id, method, params }));
  });
}
async function evaluate(expression) {
  const result = await call('Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise: true,
  });
  if (result.exceptionDetails)
    throw new Error(JSON.stringify(result.exceptionDetails));
  return result.result.value;
}
async function settle() {
  await evaluate(
    'new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))',
  );
}
async function visit(page, width = 1440, theme = 'light') {
  await call('Emulation.setDeviceMetricsOverride', {
    width,
    height: width < 600 ? 844 : 1000,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await evaluate(`location.hash = ${JSON.stringify(page)}`);
  await new Promise((resolve) => setTimeout(resolve, 30));
  await evaluate(
    `if(document.documentElement.dataset.theme !== ${JSON.stringify(theme)}) document.querySelector('.design-controls .icon-button').click()`,
  );
  await settle();
}
async function screenshot(name, full = true) {
  // Transient feedback from the previous interaction should not pollute a plate.
  await evaluate(
    `document.querySelector('.toast')?.style.setProperty('visibility', 'hidden')`,
  );
  const { cssContentSize } = await call('Page.getLayoutMetrics');
  const params = { format: 'png', captureBeyondViewport: full };
  if (full)
    params.clip = {
      x: 0,
      y: 0,
      width: cssContentSize.width,
      height: cssContentSize.height,
      scale: 1,
    };
  const result = await call('Page.captureScreenshot', params);
  await writeFile(
    new URL(`screenshots/${name}.png`, root),
    Buffer.from(result.data, 'base64'),
  );
  await evaluate(
    `document.querySelector('.toast')?.style.removeProperty('visibility')`,
  );
}
async function click(selector) {
  await evaluate(
    `(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (!el) throw new Error('Missing selector: '+${JSON.stringify(selector)}); el.click(); })()`,
  );
  await settle();
}
async function clickText(selector, text) {
  await evaluate(
    `(() => { const el = [...document.querySelectorAll(${JSON.stringify(selector)})].find(el => el.textContent.trim().includes(${JSON.stringify(text)})); if (!el) throw new Error('Missing control: '+${JSON.stringify(text)}); el.click(); })()`,
  );
  await settle();
}
async function fill(selector, value) {
  await evaluate(
    `(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (!el) throw new Error('Missing input'); el.value = ${JSON.stringify(value)}; el.dispatchEvent(new Event('input', { bubbles: true })); el.dispatchEvent(new Event('change', { bubbles: true })); })()`,
  );
  await settle();
}
async function check(name, expression) {
  assert.equal(await evaluate(expression), true, name);
  interactionChecks.push(name);
}

try {
  await call('Page.enable');
  await call('Runtime.enable');
  await call('Network.enable');
  await call('Network.setCacheDisabled', { cacheDisabled: true });
  await call('Page.navigate', {
    url: base + '?review=' + Date.now() + '#overview',
  });
  for (let attempt = 0; attempt < 40; attempt++) {
    if (await evaluate('!!document.querySelector("h1")')) break;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  await check('Offline page mounts', '!!document.querySelector("h1")');
  await mkdir(new URL('screenshots/', root), { recursive: true });
  const pages = [
    'overview',
    'users',
    'projects',
    'devices',
    'sync',
    'audit',
    'settings',
    'login',
    'register',
    'specs',
  ];
  for (const width of [1440, 1280, 390]) {
    for (const theme of ['light', 'dark']) {
      for (const page of pages) {
        await visit(page, width, theme);
        const layout = await evaluate(`({
          overflow: document.documentElement.scrollWidth > innerWidth,
          h1: document.querySelectorAll('h1').length,
          missingText: /(?:auditAction|nav|title|description|status|common)\\.[a-zA-Z]/.test(document.body.innerText),
          brokenImages: [...document.images].some(image => !image.complete || !image.naturalWidth),
          emptyButtons: [...document.querySelectorAll('button')].filter(button => button.getClientRects().length && !button.textContent.trim() && !button.getAttribute('aria-label')).length
        })`);
        layoutChecks.push({ page, width, theme, ...layout });
        assert.equal(
          layout.overflow,
          false,
          `Horizontal overflow: ${page}/${width}/${theme}`,
        );
        assert.equal(layout.h1, 1, `Missing main title: ${page}`);
        assert.equal(layout.missingText, false, `Untranslated text: ${page}`);
        assert.equal(layout.brokenImages, false, `Missing asset: ${page}`);
        assert.equal(layout.emptyButtons, 0, `Unnamed button: ${page}`);
        if (
          width === 1440 ||
          (width === 1280 &&
            ['overview', 'users', 'projects'].includes(page)) ||
          (width === 390 && page === 'overview' && theme === 'light')
        )
          await screenshot(`${page}-${width}-${theme}`);
      }
    }
  }

  await visit('users');
  await fill('.search-field input', 'lin@example.test');
  await check(
    'User search filters rows',
    'document.querySelectorAll("tbody tr").length === 1 && document.querySelector("tbody").innerText.includes("lin@example.test")',
  );
  await fill('.search-field input', 'no-matching-account');
  await check(
    'Search has an empty result state',
    '!!document.querySelector(".inline-empty")',
  );
  await fill('.search-field input', '');
  await clickText('.list-tabs button', '待审核');
  await check(
    'Pending account tab',
    'document.querySelectorAll("tbody tr").length === 2',
  );
  await clickText('tbody button', '批准');
  await fill('dialog[open] textarea', '审核通过，仅用于界面演示');
  await click('dialog[open] .modal-footer .primary');
  await check(
    'Account approval removes pending record',
    'document.querySelectorAll("tbody tr").length === 1',
  );
  await clickText('.list-tabs button', '全部');
  await click('.pagination button:last-child');
  await check(
    'Pagination advances',
    'document.querySelector(".pagination").innerText.includes("第 2 页")',
  );
  await click('.pagination button:first-child');
  await clickText('.heading-actions button', '新建用户');
  await fill('dialog[open] input[type=email]', 'demo-review@example.test');
  await fill('dialog[open] input[type=password]', 'DemoReview1234');
  await fill('dialog[open] label:last-of-type input', 'DemoReview1234');
  await click('dialog[open] .modal-footer .primary');
  await check(
    'New account appears in list',
    'document.querySelector("tbody").innerText.includes("demo-review@example.test")',
  );
  await clickText('tbody button', '禁用');
  await fill('dialog[open] textarea', '演示停用确认流程');
  await fill('dialog[open] input', 'wrong-id');
  await click('dialog[open] .modal-footer .danger-button');
  await check(
    'Disable requires exact target ID',
    '!!document.querySelector("dialog[open] .form-error")',
  );
  await screenshot('user-disable-1440-light', false);
  await click('dialog[open] .modal-header .icon-button');

  await visit('projects');
  await click('tbody .identity-button');
  await check('Project drawer opens', 'document.querySelector(".drawer").open');
  await screenshot('project-detail-1440-light', false);
  await click('dialog[open] .drawer-footer .primary');
  await fill('dialog[open] input[type=number]', '8200');
  await fill('dialog[open] textarea', '应用户请求恢复到历史版本');
  await screenshot('restore-target-1440-light', false);
  await click('dialog[open] .modal-footer .primary');
  await fill('dialog[open] input', 'wrong-project');
  await click('dialog[open] .modal-footer .primary');
  await check(
    'Restore requires exact project ID',
    '!!document.querySelector("dialog[open] .form-error")',
  );
  const projectId = await evaluate(
    'document.querySelector("dialog[open] .target-card small").textContent.trim()',
  );
  await fill('dialog[open] input', projectId);
  await settle();
  await screenshot('restore-confirm-1440-light', false);
  await evaluate(
    "document.querySelector('.design-controls .icon-button').click()",
  );
  await settle();
  await screenshot('restore-confirm-1440-dark', false);
  await click('dialog[open] .modal-footer .primary');
  await new Promise((resolve) => setTimeout(resolve, 50));
  await check(
    'Restore is queued and opens task tab',
    'location.hash === "#sync" && document.querySelector("tbody").innerText.includes("排队中") && document.querySelector(".list-tabs .active").textContent.includes("恢复任务")',
  );
  await click('tbody .icon-button');
  await check(
    'Restore task details show status, no fake percent',
    'document.querySelector(".task-status").textContent.includes("排队中") && !document.querySelector("dialog[open]").innerText.includes("100%")',
  );
  await click('dialog[open] .drawer-header .icon-button');

  await visit('settings');
  await click('.switch');
  await click('dialog[open] .modal-footer .primary');
  await check(
    'Registration switch applies confirmation',
    'document.querySelector(".switch").getAttribute("aria-checked") === "false"',
  );
  await visit('register');
  await check(
    'Closed registration has a dedicated state',
    'document.querySelector(".auth-card").innerText.includes("自助注册暂未开放")',
  );
  await screenshot('registration-closed-1440-light');
  await visit('settings');
  await click('.switch');
  await click('dialog[open] .modal-footer .primary');
  await visit('register');
  await fill('.auth-card input[type=email]', 'register-demo@example.test');
  await fill('.auth-card .password-field input', 'RegisterDemo1234');
  await fill('.auth-card label:last-of-type input', 'RegisterDemo1234');
  await click('.auth-submit');
  await check(
    'Registration enters approval state',
    'document.querySelector(".auth-card").innerText.includes("申请已提交")',
  );
  await screenshot('registration-success-1440-light');
  await visit('login');
  await fill('.auth-card input[type=email]', 'admin@example.test');
  await fill('.auth-card input[type=password]', 'ExamplePassword123');
  await click('.auth-submit');
  await new Promise((resolve) => setTimeout(resolve, 50));
  await check('Demo login opens overview', 'location.hash === "#overview"');

  for (const state of ['loading', 'empty', 'error']) {
    await fill('#preview-state', state);
    await check(
      `${state} state is rendered`,
      '!!document.querySelector(".state-panel")',
    );
    await screenshot(`state-${state}-1440-light`);
  }
  await fill('#preview-state', 'normal');
  await clickText('.segmented button', '1 天');
  await check(
    'One-day trend preserves daily granularity',
    'document.querySelectorAll(".trend-chart circle").length === 1',
  );
  await clickText('.segmented button', '30 天');
  await check(
    'Thirty-day trend changes data points',
    'document.querySelectorAll(".trend-chart circle").length === 30',
  );
  assert.equal(errors.length, 0, 'Browser errors');
  assert.equal(
    network.filter((url) => /^https?:/.test(url)).length,
    0,
    'Prototype must not contact remote services',
  );
  const result = {
    layoutChecks,
    interactionChecks,
    errors,
    remoteRequests: [],
    screenshotDirectory: 'screenshots/',
    reviewDate: new Date().toISOString(),
  };
  await writeFile(
    new URL('review-results.json', root),
    JSON.stringify(result, null, 2) + '\n',
  );
  console.log(
    JSON.stringify(
      {
        layouts: layoutChecks.length,
        interactions: interactionChecks.length,
        errors: errors.length,
        remoteRequests: 0,
        output: fileURLToPath(root),
      },
      null,
      2,
    ),
  );
} finally {
  socket.close();
}
