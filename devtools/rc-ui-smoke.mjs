// devtools/rc-ui-smoke.mjs —— 报销操作台栏目的端到端冒烟
//
// 覆盖：侧栏导航行 → 栏目切换 → 服务状态 → 经 /api/reimburse 反代取环境数据
//        → 目录浏览（真读本机）→ 红线守卫（页面不得有「提交」入口）→ 老栏目回归 → 控制台干净
//
// 依赖：应用要在运行（127.0.0.1:8766）。浏览器借用技能目录里的 playwright，
//       技能路径从仓库根的 reimburse.json 的 skillRoot 读，不写死。
//
// 用法：node devtools/rc-ui-smoke.mjs              默认 http://127.0.0.1:8766
//       node devtools/rc-ui-smoke.mjs <url>        换地址
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..');
const URL = process.argv[2] || 'http://127.0.0.1:8766';
const SHOT = path.join(HERE, 'preview-rc-section.png');

const cfgPath = path.join(ROOT, 'reimburse.json');
if (!fs.existsSync(cfgPath)) { console.error('❌ 找不到 reimburse.json：' + cfgPath); process.exit(2); }
const cfg = JSON.parse(fs.readFileSync(cfgPath, 'utf8'));
const skillRoot = cfg.skillRoot;
let chromium;
try {
  const req = createRequire(path.join(skillRoot, 'runner', '_.js'));
  ({ chromium } = req('playwright'));
} catch (e) {
  console.error('❌ 加载 playwright 失败（需要技能目录里的 runner/node_modules）。');
  console.error('   skillRoot = ' + skillRoot);
  console.error('   ' + e.message);
  process.exit(2);
}

const errs = [], failedReq = [];
let pass = 0, fail = 0;
const ok = (c, m) => { c ? (pass++, console.log('  ✅ ' + m)) : (fail++, console.log('  ❌ ' + m)); };

const browser = await chromium.launch({ channel: 'chrome', headless: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
page.on('console', (m) => { if (m.type() === 'error') errs.push(m.text()); });
page.on('pageerror', (e) => errs.push('PAGEERROR: ' + e.message));
page.on('response', (r) => { if (r.status() >= 400) failedReq.push(r.status() + ' ' + r.url()); });

try {
  console.log('=== 1. 打开指挥台 ===');
  await page.goto(URL, { waitUntil: 'domcontentloaded' });
  ok(await page.locator('#desktop-nav').isVisible(), '页面加载 + 侧栏可见');

  console.log('=== 2. 侧栏「报销操作台」行 ===');
  const navRow = page.locator('[data-target="reimburse"]').first();
  ok(await navRow.count() > 0, '导航行存在');
  ok((await navRow.innerText()).includes('报销操作台'), '文案 = 报销操作台');

  console.log('=== 3. 切换栏目 ===');
  await navRow.click();
  await page.waitForTimeout(400);
  ok(await page.locator('[data-section="reimburse"]').isVisible(), '栏目已显示');
  ok((await page.locator('#page-title').innerText()).includes('报销操作台'), '顶栏标题已切换');
  ok(await page.locator('.rc-redline').isVisible(), '红线横幅可见');

  console.log('=== 4. 服务状态 + 环境数据（经 /api/reimburse 反代）===');
  await page.waitForFunction(() => {
    const c = document.getElementById('rcSvcChip');
    return c && !/未知|正在启动/.test(c.textContent);
  }, { timeout: 30000 }).catch(() => {});
  const svcText = (await page.locator('#rcSvcChip').innerText()).trim();
  ok(/已运行/.test(svcText), '服务状态：' + svcText);
  if (!/已运行/.test(svcText)) console.log('     （页面每次进栏目会自动拉起服务；若一直起不来，先跑 node web/server.mjs 看报错）');

  await page.waitForFunction(() => {
    const c = document.getElementById('envChips');
    return c && !/加载中/.test(c.textContent);
  }, { timeout: 30000 }).catch(() => {});
  const chips = (await page.locator('#envChips').innerText()).replace(/\s+/g, ' ');
  console.log('  chips: ' + chips);
  ok(/报销说明/.test(chips) && /出差人/.test(chips) && /分摊部门/.test(chips) && /伙食费/.test(chips), '白名单 4 列已渲染');
  ok(/真源\s*\d{4}-\d{2}-\d{2}/.test(chips), '真源版本已渲染');

  console.log('=== 5. 工单 / 批次 / 表单 ===');
  ok(await page.locator('#f_job option').count() > 0, '工单下拉有 ' + await page.locator('#f_job option').count() + ' 项');
  ok(await page.locator('#f_travelType option').count() > 0, '出差类型下拉已填充（来自真源 judgement）');

  console.log('=== 6. 目录浏览（真读本机）===');
  const home = process.env.USERPROFILE || 'C:\\';
  const browseRoot = path.join(home, 'Desktop');
  await page.fill('#browsePath', browseRoot);
  await page.locator('button:has-text("打开")').first().click();
  await page.waitForTimeout(1800);
  const fileCount = await page.locator('#files .rc-file').count();
  if (fs.existsSync(browseRoot)) {
    ok(fileCount > 0, `附件归类预览渲染了 ${fileCount} 个条目`);
    ok(await page.locator('#files .rc-tag').count() === fileCount, '每个条目都有归类标签');
    await page.locator('button:has-text("用这个目录")').click();
    ok((await page.locator('#chosenDir').innerText()).includes('已选'), '「用这个目录」已生效');
  } else {
    console.log(`  ⏭  跳过（${browseRoot} 不存在）`);
  }

  console.log('=== 7. 红线守卫：不得存在「提交」入口（K1）===');
  ok(await page.locator('button:has-text("提交")').count() === 0, '页面上没有任何「提交」按钮');

  console.log('=== 8. 老栏目回归 ===');
  for (const [sec, label] of [['console', '今日指挥舱'], ['agents', '智能体模块'], ['tasks', '任务派发'], ['plugins', '插件舱']]) {
    await page.locator(`[data-target="${sec}"]`).first().click();
    await page.waitForTimeout(180);
    const vis = await page.locator(`[data-section="${sec}"]`).isVisible();
    const title = await page.locator('#page-title').innerText();
    ok(vis && title.includes(label), `${label} 仍可切换`);
  }

  await page.locator('[data-target="reimburse"]').first().click();
  await page.waitForTimeout(600);
  await page.screenshot({ path: SHOT });
  console.log('  截图 → ' + SHOT);

  console.log('=== 9. 控制台 / 网络 ===');
  ok(errs.length === 0, errs.length ? '控制台错误：' + errs.slice(0, 5).join(' | ') : '无 JS 错误');
  ok(failedReq.length === 0, failedReq.length ? '失败请求：' + failedReq.slice(0, 5).join(' | ') : '无失败请求');
} finally {
  await browser.close();
}

console.log(`\n=== 结果: ${pass} passed, ${fail} failed ===`);
process.exit(fail === 0 ? 0 : 1);
