/**
 * 无头 Chromium 预览：不开 GUI、不碰 WebView2，直接渲染页面并截图。
 *
 * 为什么需要它：这台机器上从 WorkBuddy 启动 Tauri 应用会被沙箱卡死（见《启动卡死-交接文档》），
 * 所以"看界面"只能靠两条路 —— ① 用户手动启动后抓屏 ② 本脚本用 Chromium 渲染。
 * Chromium 与 WebView2 同内核，样式结论可直接采信。
 *
 * 用法：
 *   node devtools/preview.cjs pet            渲染 pet.html：余量气泡 + 台词气泡
 *   node devtools/preview.cjs net            渲染 net.html：总速 + 明细 + 左右翻边
 *   node devtools/preview.cjs pet net        两个都渲染
 *
 * 产出：<项目根>/preview-<页名>-<状态>.png
 */
const path = require('path');
const fs = require('fs');

const ROOT = path.join(__dirname, '..');
const PUB = path.join(ROOT, 'public');

/* 自动定位已安装的 chromium-headless-shell（playwright 默认按自身版本号找，常对不上） */
function findChromium() {
  const base = path.join(process.env.LOCALAPPDATA || '', 'ms-playwright');
  if (!fs.existsSync(base)) throw new Error('找不到 ms-playwright 目录: ' + base);
  const dirs = fs.readdirSync(base)
    .filter(d => d.startsWith('chromium_headless_shell-'))
    .sort()
    .reverse();
  for (const d of dirs) {
    const exe = path.join(base, d, 'chrome-headless-shell-win64', 'chrome-headless-shell.exe');
    if (fs.existsSync(exe)) return exe;
  }
  const full = fs.readdirSync(base).filter(d => d.startsWith('chromium-')).sort().reverse();
  for (const d of full) {
    const exe = path.join(base, d, 'chrome-win64', 'chrome.exe');
    if (fs.existsSync(exe)) return exe;
  }
  throw new Error('没找到可用的 chromium 可执行文件');
}

/* 假接口数据：让页面显示真实形态的文案（不是全 '—'） */
function fakeApi(netSide) {
  const T = { status: 200, contentType: 'application/json', body: JSON.stringify({
    ok: true, down: 1048576 * 5.11, up: 1024 * 142,
    downText: '5.11 M/s', upText: '142 K/s',
    warm: false, stale: false, age: 0.4, scope: 'all',
    enabled: true, gapLp: -18, side: netSide, sampleMs: 1000,
    ifaces: [
      { name: '以太网 6', down: 1048576 * 5.11, up: 1024 * 142, downText: '5.11 M/s', upText: '142 K/s' },
      { name: 'WLAN', down: 1024 * 88, up: 1024 * 12, downText: '88 K/s', upText: '12 K/s' }
    ]
  }) };
  return {
    '/api/usage': { ok: true, usage: { monthly: { percent: 27 }, rolling: { percent: 4 }, weekly: { percent: 9 } } },
    '/api/dshbalance': { ok: true, isAvailable: true, balances: [{ currency: 'CNY', total_balance: '0.14' }] },
    '/api/state': { tasks: [{ status: 'doing' }] },
    '/api/netspeed': T,
    '/api/netspeed/detail': T
  };
}

async function shoot(chromium, page, out, opt) {
  const b = await chromium.launch({ executablePath: findChromium() });
  /* net.html 会被嵌进 166x56 的窗口；pet.html 是 204x280。留大一点好看清周边 */
  const size = opt.viewport || { width: 204, height: 300 };
  const p = await b.newPage({ viewport: size, deviceScaleFactor: 3 });
  const api = fakeApi(opt.side || 'right');
  await p.route('**/*', route => {
    const u = new URL(route.request().url());
    if (u.pathname in api) return route.fulfill(api[u.pathname]);
    const f = path.join(PUB, decodeURIComponent(u.pathname.replace(/^\//, '')));
    if (fs.existsSync(f) && fs.statSync(f).isFile()) return route.fulfill({ path: f });
    return route.fulfill({ status: 404, body: 'nf' });
  });
  await p.goto('http://local.test/' + page, { waitUntil: 'domcontentloaded' });
  await p.waitForTimeout(1600);
  if (opt.clickSel) {
    const box = await p.locator(opt.clickSel).boundingBox();
    if (box) await p.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await p.waitForTimeout(400);
  }
  const file = path.join(ROOT, out);
  await p.screenshot({ path: file });
  const state = await p.evaluate(() => {
    const g = id => { const e = document.getElementById(id); return e ? (e.textContent || '').trim() : '(缺)'; };
    return {
      p1: g('stGo') + ' / ' + g('stDs'),
      n1: g('lb1') + ' ' + g('vl1'),
      n2: g('lb2') + ' ' + g('vl2'),
      msg: g('msgTxt')
    };
  });
  console.log('  ' + out + '   ' + JSON.stringify(state));
  await b.close();
}

(async () => {
  const { chromium } = require('playwright-core');
  const want = process.argv.slice(2).filter(a => a === 'pet' || a === 'net');
  const pages = want.length ? want : ['pet', 'net'];
  for (const pg of pages) {
    if (pg === 'pet') {
      await shoot(chromium, 'pet.html', 'preview-pet-status.png', { viewport: { width: 210, height: 300 } });
      await shoot(chromium, 'pet.html', 'preview-pet-quip.png', { viewport: { width: 210, height: 300 }, clickSel: '#pet' });
    } else {
      await shoot(chromium, 'net.html', 'preview-net-total.png', { viewport: { width: 180, height: 70 } });
      await shoot(chromium, 'net.html', 'preview-net-detail.png', { viewport: { width: 180, height: 70 }, clickSel: '#wrap' });
      await shoot(chromium, 'net.html', 'preview-net-flipped.png', {
        viewport: { width: 180, height: 70 }, side: 'left'
      });
    }
  }
})().catch(e => { console.error('ERR', e.message); process.exit(1); });
