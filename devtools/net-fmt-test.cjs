/**
 * net.html 显示口径测试（Node + DOM 桩，不用启动应用）
 *
 * 为什么需要：网速的"值 → 文字"有两份实现 ——
 *   Rust  side: server.rs::fmt_speed()   （给 API 消费者，反映瞬时值）
 *   JS    side: net.html::fmt()          （给界面，反映 EMA 平滑后的值）
 * 两份必须同口径，否则接口和界面会各说各话。这个测试把 JS 那份钉死。
 *
 * 用法：node devtools/net-fmt-test.cjs
 */
const fs = require('fs');
const path = require('path');

const ROOT = path.join(__dirname, '..');
const html = fs.readFileSync(path.join(ROOT, 'public', 'net.html'), 'utf8');
const js = html.match(/<script>([\s\S]*?)<\/script>/)[1];

/* ---------- 最小 DOM 桩 ---------- */
function mkEl(id) {
  return {
    id, textContent: '', innerHTML: '', className: '',
    addEventListener() { },
    classList: { add() { }, remove() { }, contains() { return false; } }
  };
}
const els = {};
global.document = {
  getElementById(id) { return els[id] || (els[id] = mkEl(id)); }
};
global.window = {};
global.fetch = () => new Promise(() => { });      // 挂起，避免轮询干扰
global.setInterval = () => 0;
global.setTimeout = () => 0;

/* ---------- 执行引擎 ---------- */
new Function(js)();

const fmt = global.window.__netFmt;
const level = global.window.__netLevel;

let pass = 0, fail = 0;
function check(name, ok, extra) {
  if (ok) { pass++; console.log('  \u2713 ' + name); }
  else { fail++; console.log('  \u2717 ' + name + (extra ? '  \u2192 ' + extra : '')); }
}
const K = 1024;

console.log('== 1. 测试缝暴露 ==');
check('net.html 暴露了 __netFmt / __netLevel', typeof fmt === 'function' && typeof level === 'function');

console.log('\n== 2. 格式化（与 Rust fmt_speed 同口径）==');
const cases = [
  [0, '0 K/s'],
  [0.4, '0 K/s'],
  [512, '512 B/s'],
  [1023, '1023 B/s'],
  [1024, '1 K/s'],
  [1024 * 412, '412 K/s'],
  [1024 * 1024, '1.00 M/s'],
  [1024 * 1024 * 5.108, '5.11 M/s'],   // 实测那次限速下载的读数
  [1024 * 1024 * 99.9, '99.90 M/s'],
  [1024 * 1024 * 120, '120.0 M/s'],    // ≥100 降到一位小数
  [1024 * 1024 * 1024, '1024.0 M/s']
];
for (const [bps, want] of cases) {
  const got = fmt(bps);
  check(`${bps} B/s \u2192 ${want}`, got === want, '得到 ' + got);
}

console.log('\n== 2b. 异常输入不能渲染出 NaN ==');
for (const bad of [NaN, Infinity, -Infinity, undefined, null, -1]) {
  const out = String(fmt(bad));
  check(`fmt(${String(bad)}) 不含 NaN/Infinity`, out.indexOf('NaN') < 0 && out.indexOf('Infinity') < 0, out);
}

console.log('\n== 3. 色阶阈值（空闲/活跃/高速）==');
check('7 KB/s 属空闲档', level(7 * K) === ' dim');
check('8 KB/s 已离开空闲档', level(8 * K) === '');
check('100 KB/s 属活跃档', level(100 * K) === '');
check('5 MB/s 进入高速档', level(5 * K * K) === ' fast');
check('4.9 MB/s 仍是活跃档', level(4.9 * K * K) === '');

console.log('\n== 4. 页面结构（查 HTML 源，桩不解析 DOM）==');
check('气泡初始停靠在右侧', /id="bubble"\s+class="right"/.test(html));
check('两个数值槽初始显示占位符 ——',
  /id="dn">\u2014<\/span>/.test(html) && /id="up">\u2014<\/span>/.test(html));
check('带指向宠物的小尾巴（左右两套三角）', /\bbubble\.right::before\b/.test(html) && /\bbubble\.left::after\b/.test(html));
check('声明了 1 秒轮询', /setInterval\(pull,\s*1000\)/.test(js));
check('声明了自热重载', /checkReload\(/.test(js) && /location\.reload\(\)/.test(js));

console.log('\n== 5. 与 Rust 侧同口径的锚点值 ==');
check('fmt_speed 的四个档位在 JS 侧一致（B/s 档只有 <1024 才出现）',
  fmt(1023).indexOf('B/s') > 0 && fmt(1024).indexOf('K/s') > 0
  && fmt(1024 * 1023).indexOf('K/s') > 0 && fmt(1024 * 1024).indexOf('M/s') > 0);

console.log('\n== 结果 ==');
console.log('  通过 ' + pass + ' / 失败 ' + fail);
process.exit(fail ? 1 : 0);
