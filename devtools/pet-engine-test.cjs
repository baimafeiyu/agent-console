/**
 * pet.html 动画引擎离线测试（Node + DOM 桩，不需要启动应用）
 *
 * 观测手段：引擎唯一的对外可见行为是 ctx.drawImage(img, sx, sy, ...)
 *   sy = 行号 × 208  → 反推当前在播哪条轨道
 *   sx = 帧号 × 192  → 反推播到第几帧
 * 于是"当前动作"完全可断言。
 *
 * 用法：node devtools/pet-engine-test.cjs
 */
const fs = require('fs');
const path = require('path');

const ROOT = path.join(__dirname, '..');
const html = fs.readFileSync(path.join(ROOT, 'public', 'pet.html'), 'utf8');
const js = html.match(/<script>([\s\S]*?)<\/script>/)[1];

const CELL_W = 192, CELL_H = 208;
/* 行号 → 轨道名：从 pet.json 推导（row 字段优先，缺省用键序），
   与 pet.html 的取值规则一致。不要再手工维护这张表 —— 加行动作时
   手工表会漏项，导致断言把新轨道误认成 "row9"。 */
const MANIFEST = JSON.parse(fs.readFileSync(path.join(ROOT, 'public', 'pet.json'), 'utf8'));
const ROW = {};
{
  const tracks = (MANIFEST.sprite2d && MANIFEST.sprite2d.tracks) || {};
  Object.keys(tracks).forEach((n, i) => {
    const r = (typeof tracks[n].row === 'number') ? tracks[n].row : i;
    ROW[r] = n;
  });
}
const BEH = JSON.parse(fs.readFileSync(path.join(ROOT, 'public', 'pet-behavior.json'), 'utf8'));

/* ---------------- 虚拟时钟 + 定时器 ---------------- */
let now = 0;
let timers = [];
let rafCb = null;
let timerSeq = 1;
global.performance = { now: () => now };
global.requestAnimationFrame = (cb) => { rafCb = cb; };
global.setTimeout = (fn, ms) => { const id = timerSeq++; timers.push({ id, at: now + (ms || 0), fn, every: 0 }); return id; };
global.setInterval = (fn, ms) => { const id = timerSeq++; const p = ms || 1000; timers.push({ id, at: now + p, fn, every: p }); return id; };
global.clearTimeout = (id) => { timers = timers.filter(t => t.id !== id); };

/* ---------------- DOM 桩 ---------------- */
const ctxLog = [];
const canvasCtx = {
  clearRect() { },
  drawImage(img, sx, sy) { ctxLog.push({ sx, sy, at: now }); }
};
function mkEl(id) {
  const handlers = {};
  const attrs = {};
  return {
    id, innerHTML: '', textContent: '', title: '', style: {},
    classList: {
      _s: new Set(),
      add(c) { this._s.add(c); }, remove(c) { this._s.delete(c); },
      contains(c) { return this._s.has(c); }
    },
    addEventListener(t, f) { handlers[t] = f; },
    _fire(t, e) { if (handlers[t]) handlers[t](e || {}); },
    _has(t) { return !!handlers[t]; },
    setAttribute(k, v) { attrs[k] = String(v); },
    getAttribute(k) { return (k in attrs) ? attrs[k] : null; },
    /* 余量气泡改用内联 SVG，文字靠 textContent + class，
       字号自适应靠 getComputedTextLength —— 桩里给个粗估即可 */
    getComputedTextLength() { return String(this.textContent || '').length * 11; },
    querySelector() { return mkEl('q'); },
    contains() { return false; },
    getContext() { return canvasCtx; }
  };
}
const els = {};
global.document = {
  getElementById(id) { return els[id] || (els[id] = mkEl(id)); },
  addEventListener() { },
  documentElement: { addEventListener() { } },
  body: {}
};
const winHandlers = {};
global.window = {
  addEventListener(t, f) { winHandlers[t] = f; },
  __TAURI__: null
};
let lastImg = null;
global.Image = function () {
  lastImg = this; this.complete = true; this.naturalWidth = 1536; this.onload = null;
};

/* ---------------- fetch 桩 ---------------- */
let FAKE_DOING = 0;
let FAKE_USAGE = 18;
global.fetch = (url) => {
  const u = String(url);
  let body = {};
  if (u.indexOf('/pet.json') >= 0) body = JSON.parse(fs.readFileSync(path.join(ROOT, 'public', 'pet.json'), 'utf8'));
  else if (u.indexOf('/pet-behavior.json') >= 0) body = JSON.parse(fs.readFileSync(path.join(ROOT, 'public', 'pet-behavior.json'), 'utf8'));
  else if (u.indexOf('/api/state') >= 0) {
    const tasks = [];
    for (let i = 0; i < FAKE_DOING; i++) tasks.push({ status: 'doing' });
    tasks.push({ status: 'todo' }, { status: 'done' });
    body = { tasks };
  }
  else if (u.indexOf('/api/usage') >= 0) body = { ok: true, usage: { monthly: { percent: FAKE_USAGE }, rolling: { percent: 1 }, weekly: { percent: 2 } } };
  else if (u.indexOf('/api/dshbalance') >= 0) body = { ok: true, isAvailable: true, balances: [{ currency: 'CNY', total_balance: '1.23' }] };
  return Promise.resolve({ ok: true, json: () => Promise.resolve(body) });
};

/* ---------------- 时间推进 ---------------- */
const flush = () => new Promise(r => setImmediate(r));

async function advance(ms, stepMs = 100) {
  const target = now + ms;
  while (now < target) {
    now = Math.min(target, now + stepMs);
    const due = timers.filter(t => t.at <= now);
    for (const t of due) {
      if (t.every) t.at = now + t.every; else timers = timers.filter(x => x.id !== t.id);
      t.fn();
    }
    if (rafCb) { const cb = rafCb; rafCb = null; cb(); }
    await flush();
  }
}

/* ---------------- 断言 ---------------- */
let pass = 0, fail = 0;
function check(name, ok, extra) {
  if (ok) { pass++; console.log('  \u2713 ' + name); }
  else { fail++; console.log('  \u2717 ' + name + (extra ? '  → ' + extra : '')); }
}
function currentRow() { return ctxLog.length ? ctxLog[ctxLog.length - 1].sy / CELL_H : null; }
function currentName() { const r = currentRow(); return r === null ? '(未绘制)' : (ROW[r] || 'row' + r); }
function rowSeen(name) {
  const r = Object.keys(ROW).find(k => ROW[k] === name);
  return ctxLog.some(c => c.sy / CELL_H === Number(r));
}
function frameAdvanced() {
  const last = ctxLog.slice(-14);
  return new Set(last.map(c => c.sx)).size > 1;
}

(async function main() {
  console.log('== 加载 pet.html 引擎 ==');
  // 在模块作用域执行脚本（它自带 IIFE）
  new Function(js)();
  check('脚本可执行且完成初始化', typeof lastImg === 'object' && !!lastImg);
  check('已注册 canvas 交互事件', els.pet._has('mouseenter') && els.pet._has('mousedown') && els.pet._has('contextmenu'));
  check('已注册 window 拖拽事件', typeof winHandlers.mousemove === 'function' && typeof winHandlers.mouseup === 'function');

  console.log('\n== 1. 清单加载（pet.json / pet-behavior.json）==');
  lastImg.onload();                       // 启动动画循环
  await advance(200);
  await flush();
  await advance(400);
  check('读取到 pet.json 并进入 idle（row 0）', currentName() === 'idle', currentName());
  check('帧号在推进（不是卡在单帧）', frameAdvanced());
  // 余量气泡（动漫化内联 SVG）：文字写进 stGo / stDs，色阶走 class
  //   FAKE_USAGE=18 → Go 已用 18% → 余 82%（<70 不告警 → class 仅 "v"）
  //   ds=1.23 → <5 → hot 档
  check('两行标签在 HTML 源码里（Go 本月 / DeepSeek）',
    /class="k"[^>]*>Go 本月</.test(html) && /class="k"[^>]*>DeepSeek</.test(html));
  check('余量气泡用的是 SVG 椭圆+下向尾（不是旧的 div#status）',
    /id="statusSvg"/.test(html) && !/id="status"/.test(html));
  check('墨线带手绘抖动滤镜', /feTurbulence/.test(html) && /feDisplacementMap/.test(html));
  check('Go 余量按数据算出「余 82%」', els.stGo.textContent === '余 82%', els.stGo.textContent);
  check('DeepSeek 余额写成「¥ 1.23」', els.stDs.textContent === '¥ 1.23', els.stDs.textContent);
  check('余额 <5 → hot 档', els.stDs.getAttribute('class') === 'v hot', els.stDs.getAttribute('class'));
  check('用量 18% 不告警 → 仅 "v" 类', els.stGo.getAttribute('class') === 'v', els.stGo.getAttribute('class'));
  check('余量气泡初始可见、台词气泡隐藏',
    els.statusSvg.style.display !== 'none' && els.msgSvg.style.display === 'none',
    els.statusSvg.style.display + ' / ' + els.msgSvg.style.display);

  console.log('\n== 1b. 台词气泡临时顶替余量气泡，超时后恢复 ==');
  check('默认不显示台词气泡', els.msgSvg.style.display === 'none');
  check('台词字号默认初始化为 12px', els.msgTxt.getAttribute('font-size') === '12',
    String(els.msgTxt.getAttribute('font-size')));
  els.pet._fire('mousedown', { button: 0, clientX: 10, clientY: 10 });
  winHandlers.mouseup({ button: 0 });
  await advance(200);
  check('点一下 → 台词气泡接管、余量气泡让位',
    els.statusSvg.style.display === 'none' && els.msgSvg.style.display === '',
    els.statusSvg.style.display + ' / ' + els.msgSvg.style.display);
  check('台词文字已写入单行气泡', String(els.msgTxt.textContent || '').length > 0, els.msgTxt.textContent);
  await advance(4200);
  check('超时后恢复显示余量气泡',
    els.statusSvg.style.display !== 'none' && els.msgSvg.style.display === 'none',
    els.statusSvg.style.display + ' / ' + els.msgSvg.style.display);
  await advance(2000);

  console.log('\n== 2. 悬停 → waiting（row 6）==');
  els.pet._fire('mouseenter');
  await advance(400);
  check('悬停切到 waiting', currentName() === 'waiting', currentName());
  els.pet._fire('mouseleave');
  await advance(400);
  check('移开回到 idle', currentName() === 'idle', currentName());

  console.log('\n== 3. 左键 → 动作轮换池（不再只跳一下）==');
  const CLICK_POOL = BEH.triggers.click.tracks;
  const clickTracks = [], clickQuips = [];
  for (let i = 0; i < 12; i++) {
    els.pet._fire('mousedown', { button: 0, clientX: 10, clientY: 10 });
    winHandlers.mouseup({ button: 0 });
    await advance(120);
    clickTracks.push(currentName());
    clickQuips.push(String(els.msgTxt.textContent || ''));
    await advance(4200);                  // 等这次播完（且超过连击窗口，避免叠加连击）
  }
  const uniqClick = [...new Set(clickTracks)];
  console.log('  实际点击序列：' + clickTracks.join(' → '));
  check('点击出现多种动作（≥3 种）', uniqClick.length >= 3, uniqClick.join(', '));
  check('点击动作全部来自配置的轮换池', clickTracks.every(n => CLICK_POOL.indexOf(n) >= 0), clickTracks.join(', '));
  let adjSame = 0;
  for (let i = 1; i < clickTracks.length; i++) if (clickTracks[i] === clickTracks[i - 1]) adjSame++;
  check('相邻两次不重复（avoidRepeat）', adjSame === 0, '重复了 ' + adjSame + ' 次');
  const uniqQuip = [...new Set(clickQuips.filter(Boolean))];
  check('台词随动作变化（≥3 种不同）', uniqQuip.length >= 3, uniqQuip.join(' / '));
  const quipOK = clickTracks.every((n, i) => {
    const q = clickQuips[i];
    if (!q) return true;                 // 允许某次只做动作不说台词
    const own = (BEH.triggers.click.quips || {})[n] || [];
    return own.indexOf(q) >= 0 || BEH.quips.indexOf(q) >= 0;
  });
  check('每次台词都属于该动作的专属台词池', quipOK);
  await advance(4500);
  check('点击动作播完回落常态', currentName() === 'idle', currentName());

  console.log('\n== 3b. 连点 4 次 → failed（被戳烦了）==');
  for (let i = 0; i < 3; i++) {
    els.pet._fire('mousedown', { button: 0, clientX: 10, clientY: 10 });
    winHandlers.mouseup({ button: 0 });
    await advance(100);
  }
  check('连点到第 3 下还没升级', currentName() !== 'failed', currentName());
  els.pet._fire('mousedown', { button: 0, clientX: 10, clientY: 10 });
  winHandlers.mouseup({ button: 0 });
  await advance(120);
  check('第 4 连点升级为 failed', currentName() === 'failed', currentName());
  check('台词出自"被戳烦"那一组', /别戳|再戳|够啦|不是按钮/.test(els.msgTxt.textContent), els.msgTxt.textContent);
  await advance(6000);
  check('failed 播完回落常态', currentName() === 'idle', currentName());
  els.pet._fire('mousedown', { button: 0, clientX: 10, clientY: 10 });
  winHandlers.mouseup({ button: 0 });
  await advance(120);
  check('升级后计数清零（下一击回到轮换池）', CLICK_POOL.indexOf(currentName()) >= 0, currentName());
  await advance(4500);

  console.log('\n== 3c. 长按 800ms → 摸头（review）==');
  els.pet._fire('mousedown', { button: 0, clientX: 10, clientY: 10 });
  await advance(500);
  check('按住 500ms 尚未触发摸头', currentName() !== 'review', currentName());
  await advance(400);                     // 累计 900ms > holdMs 800
  check('长按满 800ms 触发摸头（review）', currentName() === 'review', currentName());
  check('摸头台词出现', /摸头|舒服|再摸/.test(els.msgTxt.textContent), els.msgTxt.textContent);
  winHandlers.mouseup({ button: 0 });
  await advance(250);
  check('松手即回常态（不再停留在 review）', currentName() !== 'review', currentName());
  await advance(1000);

  console.log('\n== 4. 有任务在进行 → running（row 7）==');
  FAKE_DOING = 2;
  await advance(11000);
  check('检测到 doing 任务并切到 running', currentName() === 'running', currentName());

  console.log('\n== 5. 任务完成 → review（row 8）播一遍后回到 running 家族 ==');
  FAKE_DOING = 1;
  await advance(11000);
  check('进行中数量下降时播过 review', rowSeen('review'));
  await advance(6000);
  const afterReview = currentName();
  check('review 播完落到 agentWorking 家族（running/溜达）',
    ['running', 'running-right', 'running-left'].indexOf(afterReview) >= 0, afterReview);

  console.log('\n== 6. 空闲溜达 → running-right / running-left（row 1 / 2）==');
  FAKE_DOING = 0;
  const b0 = ctxLog.length;
  await advance(12000);                  // 数量下降 → review
  await advance(14000);                  // 播完并落回
  const seg0 = ctxLog.slice(b0);
  check('无任务时落回 idle（窗口内出现过 idle）', seg0.some(c => c.sy / CELL_H === 0));
  check('review 播完没有卡住', currentName() !== 'review', currentName());
  const before = ctxLog.length;
  await advance(60000);                   // 覆盖 wanderEveryIdleCycles=9 个环境态拍子
  const seg = ctxLog.slice(before);
  const sawR = seg.some(c => c.sy / CELL_H === 1);
  const sawL = seg.some(c => c.sy / CELL_H === 2);
  check('空闲时向右溜达过（running-right）', sawR);
  check('空闲时向左溜达过（running-left）', sawL);

  console.log('\n== 7. 用量越阈值 → failed（row 5）==');
  FAKE_USAGE = 95;
  await advance(61000);
  const before2 = ctxLog.length;
  await advance(60000);
  check('月用量 ≥90% 时演过 failed', ctxLog.slice(before2).some(c => c.sy / CELL_H === 5) || rowSeen('failed'));

  console.log('\n== 8. 拖动按方向换腿（row 1 / 2），且能打断一次性动作 ==');
  FAKE_USAGE = 18;                        // 先复位告警，避免残留 failed 干扰
  await advance(9000);
  const before3 = ctxLog.length;
  els.pet._fire('mousedown', { button: 0, clientX: 50, clientY: 50 });
  winHandlers.mousemove({ clientX: 60, clientY: 50, screenX: 1000 });
  winHandlers.mousemove({ clientX: 60, clientY: 50, screenX: 960 });   // 向左快速移动
  await advance(300);
  const seg3 = ctxLog.slice(before3);
  check('拖动中切到 running-left', seg3.some(c => c.sy / CELL_H === 2), currentName());
  winHandlers.mouseup({ button: 0 });
  FAKE_USAGE = 18;
  await advance(1000);

  console.log('\n== 9. 兜底：清单行号超出图集高度时不整只消失 ==');
  // 场景：先改 pet.json 加了新行动作，应用还没重启（图集仍是旧的 9 行）。
  // 此时 drawImage 取 sy=1872 越界 → 浏览器什么也不画，而 clearRect 已经清了画布
  // → 宠物整只消失。draw() 里的越界回落必须把它挡成 idle 行。
  const clickOnce = async () => {
    els.pet._fire('mousedown', { button: 0, clientX: 10, clientY: 10 });
    winHandlers.mouseup({ button: 0 });
    await advance(120);
  };

  // 9a. 图集升级到 10 行 → drink(row 9) 能正常绘制
  lastImg.naturalHeight = 2080;
  ctxLog.length = 0;
  let sawDrink = false;
  for (let i = 0; i < 40 && !sawDrink; i++) {
    await clickOnce();
    if (currentRow() === 9) sawDrink = true;
    await advance(4200);
  }
  check('图集 10 行时 drink(row 9) 正常绘制', sawDrink);

  // 9b. 图集退回 9 行 → 不得出现越界行，行号必须全部落在 0..8
  lastImg.naturalHeight = 1872;
  ctxLog.length = 0;
  for (let i = 0; i < 40; i++) { await clickOnce(); await advance(4200); }
  const overs = ctxLog.filter(c => c.sy + CELL_H > 1872);
  check('图集 9 行时无越界绘制（不会整只消失）', overs.length === 0,
        '越界 ' + overs.length + ' 次，如 sy=' + (overs[0] && overs[0].sy));
  check('图集 9 行时行号全部落在 0..8', ctxLog.length > 0 && ctxLog.every(c => c.sy >= 0 && c.sy <= 8 * CELL_H));
  lastImg.naturalHeight = 2080;            // 复位，避免影响后续
  await advance(4200);

  console.log('\n== 10. 兜底：配置缺失时仍能跑 ==');
  // 用一个只有契约默认值的环境重跑一份引擎实例（fetch 全部失败）
  const savedFetch = global.fetch;
  global.fetch = () => Promise.reject(new Error('offline'));
  ctxLog.length = 0;
  const els2 = {};
  const savedGet = global.document.getElementById;
  global.document.getElementById = (id) => els2[id] || (els2[id] = mkEl(id));
  new Function(js)();
  lastImg.onload();
  await advance(600);
  global.document.getElementById = savedGet;
  global.fetch = savedFetch;
  check('配置读不到时仍按契约默认播 idle（不崩）', ctxLog.some(c => c.sy / CELL_H === 0));

  console.log('\n== 结果 ==');
  console.log('  通过 ' + pass + ' / 失败 ' + fail);
  console.log('  共产生 ' + ctxLog.length + ' 次绘制调用');
  process.exit(fail ? 1 : 0);
})();
