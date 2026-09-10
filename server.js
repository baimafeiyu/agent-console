'use strict';
/**
 * agent-console 桥接服务（零依赖，仅用 Node 内置模块）
 *   node server.js [port]
 *
 * 职责：
 *   1. 托管工作台页面 public/index.html
 *   2. 探测各智能体的运行状态（进程 / 端口 / 窗口标题）
 *   3. 把「唤起并置顶某个智能体窗口」这类 OS 能力暴露成 HTTP 接口
 *      —— 浏览器里的纯 HTML 无权操作系统窗口，必须经由本机桥接
 */

const http = require('http');
const fs = require('fs');
const path = require('path');
const net = require('net');
const os = require('os');
const { spawn, execFile } = require('child_process');

const ROOT = __dirname;
const PUBLIC_DIR = path.join(ROOT, 'public');
const STATE_FILE = path.join(ROOT, 'state.json');
const AGENTS_FILE = path.join(ROOT, 'agents.json');
const WIN_PS1 = path.join(ROOT, 'win.ps1');
const DSH_PKG = path.join(os.homedir(), '.dsh', 'profiles', 'web', 'package.json');

const PORT = Number(process.argv[2] || process.env.AGENT_CONSOLE_PORT || 8765);

/* ------------------------------ 基础工具 ------------------------------ */

function isoDate(offsetDays) {
  const d = new Date(Date.now() + offsetDays * 86400000);
  return d.getFullYear() + '-' + String(d.getMonth() + 1).padStart(2, '0') + '-' + String(d.getDate()).padStart(2, '0');
}

function readJson(file, fallback) {
  try { return JSON.parse(fs.readFileSync(file, 'utf8')); } catch (e) { return fallback; }
}

function writeJson(file, data) {
  fs.writeFileSync(file, JSON.stringify(data, null, 2), 'utf8');
}

function defaultState() {
  return {
    tasks: [
      { id: 't1', title: '核对台州烟草空调维护报价（2027-2029）三档单价', agent: 'workbuddy', due: isoDate(-2), status: 'todo', note: '跨期混合报价，必须拆到每档单独核算，禁止正则解析' },
      { id: 't2', title: 'smart-reimburse 水费填报模板适配宜搭上传规范', agent: 'opencode', due: isoDate(0), status: 'doing', note: 'xls 列序需与平台模板一致' },
      { id: 't3', title: '给 dsh 装一个 PDF 解析插件', agent: 'dsh', due: isoDate(0), status: 'todo', note: 'dsh plugin --profile web add <包名>' },
      { id: 't4', title: '复盘：半导体材料主线板块地位判定', agent: 'workbuddy', due: isoDate(1), status: 'todo', note: '先画板块时间线，再定龙头/补涨' },
      { id: 't5', title: '验证 ACP 端口 8899 能否被外部客户端驱动', agent: 'opencode-acp', due: isoDate(3), status: 'todo', note: '' }
    ],
    bindings: {},
    cleared: false
  };
}

let state = readJson(STATE_FILE, null);
if (!state || typeof state !== 'object' || !Array.isArray(state.tasks)) {
  state = defaultState();
  writeJson(STATE_FILE, state);
}
function saveState() { writeJson(STATE_FILE, state); }

/* --------------------------- 窗口层（PowerShell） --------------------------- */

function runWin(action, extra) {
  return new Promise((resolve) => {
    const args = ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
      '-File', WIN_PS1, '-Action', action].concat(extra || []);
    execFile('powershell.exe', args, { windowsHide: true, maxBuffer: 8 * 1024 * 1024, timeout: 20000 },
      (err, stdout) => {
        if (err) return resolve(null);
        try { resolve(JSON.parse(String(stdout).trim())); } catch (e) { resolve(null); }
      });
  });
}

let winCache = { at: 0, list: [] };
async function getWindows(force) {
  if (!force && Date.now() - winCache.at < 2500) return winCache.list;
  const r = await runWin('list');
  const list = Array.isArray(r) ? r : [];
  winCache = { at: Date.now(), list };
  return list;
}

function checkPort(port) {
  return new Promise((resolve) => {
    if (!port) return resolve(false);
    const sock = net.createConnection({ host: '127.0.0.1', port });
    sock.setTimeout(800);
    sock.on('connect', () => { sock.destroy(); resolve(true); });
    sock.on('timeout', () => { sock.destroy(); resolve(false); });
    sock.on('error', () => { sock.destroy(); resolve(false); });
  });
}

/* --------------------- 进程探测（tasklist，不依赖 PowerShell） ---------------------
   为什么不用 PowerShell：Get-Process 在受限沙箱/策略下可能被拦截，
   而 tasklist 是纯 Windows CLI，可用性更高。窗口标题和置顶仍然只能走 PowerShell。 */

function runTasklist() {
  return new Promise((resolve) => {
    execFile('tasklist', ['/FO', 'CSV', '/NH'], { windowsHide: true, maxBuffer: 16 * 1024 * 1024, timeout: 15000 },
      (err, stdout) => {
        if (err) return resolve(null);
        const map = {};
        String(stdout).split(/\r?\n/).forEach((line) => {
          const m = line.match(/^"([^"]+)","(\d+)"/);
          if (!m) return;
          const name = m[1].toLowerCase();
          const pid = Number(m[2]);
          if (!map[name]) map[name] = [];
          map[name].push(pid);
        });
        resolve(map);
      });
  });
}

let procCache = { at: 0, map: {} };
async function getProcMap(force) {
  if (!force && Date.now() - procCache.at < 2500) return procCache.map;
  const map = (await runTasklist()) || {};
  procCache = { at: Date.now(), map };
  return map;
}

function pidAlive(procMap, pid) {
  if (!pid) return false;
  return Object.keys(procMap).some((k) => procMap[k].indexOf(Number(pid)) >= 0);
}

/* ------------------------------ 状态聚合 ------------------------------ */

async function buildAgents() {
  const cfg = readJson(AGENTS_FILE, { agents: [] });
  const agents = Array.isArray(cfg.agents) ? cfg.agents : [];
  const wins = await getWindows();
  const procMap = await getProcMap();
  const ports = await Promise.all(agents.map((a) => checkPort(a.detect && a.detect.port)));

  return agents.map((a, i) => {
    const det = a.detect || {};
    const portUp = !!ports[i];

    // 1) 进程名探测
    const pName = det.process || '';
    const procHits = pName ? (procMap[pName.toLowerCase()] || []).slice() : [];

    // 2) pidFile 探测（dsh 会把自己的 pid 写进 ~/.dsh/dsh-process.json）
    let pidFilePid = 0;
    if (det.pidFile) {
      const p = String(det.pidFile).replace(/^~/, os.homedir());
      const meta = readJson(p, null);
      if (meta && meta.pid) pidFilePid = Number(meta.pid);
    }
    const pidFileAlive = pidFilePid ? pidAlive(procMap, pidFilePid) : false;

    // 3) 窗口标题探测（依赖 PowerShell，可能被策略拦截）
    const titleHit = a.titleHint
      ? wins.find((w) => String(w.title || '').toLowerCase().includes(String(a.titleHint).toLowerCase()))
      : null;

    const running = portUp || procHits.length > 0 || !!titleHit || pidFileAlive;

    const boundPid = (state.bindings && state.bindings[a.id]) || 0;
    const boundAlive = boundPid ? pidAlive(procMap, boundPid) : false;

    const pid = boundAlive ? boundPid
      : (pidFileAlive ? pidFilePid
        : (titleHit ? titleHit.pid
          : (procHits[0] || 0)));

    const doing = state.tasks.filter((t) => t.agent === a.id && t.status === 'doing');
    const todo = state.tasks.filter((t) => t.agent === a.id && t.status === 'todo');

    return {
      id: a.id, name: a.name, subtitle: a.subtitle, role: a.role, kind: a.kind,
      color: a.color, url: a.url || '', strength: a.strength || '', caveat: a.caveat || '',
      titleHint: a.titleHint || '',
      dispatch: (a.dispatch && a.dispatch.mode) || 'clipboard',
      running, portUp, port: (a.detect && a.detect.port) || 0, pid, bound: boundAlive,
      procCount: procHits.length,
      windowTitle: titleHit ? titleHit.title : '',
      doingCount: doing.length, todoCount: todo.length,
      currentTask: doing[0] ? doing[0].title : (todo[0] ? todo[0].title : '')
    };
  });
}

function readDshPlugins() {
  const pkg = readJson(DSH_PKG, null);
  if (!pkg) return { bundles: [], available: false };
  const bundles = (pkg.dsh && pkg.dsh.profile && pkg.dsh.profile.bundles) || [];
  const deps = pkg.dependencies || {};
  return { bundles, deps, available: true };
}

/* ------------------------------- HTTP ------------------------------- */

function sendJson(res, code, obj) {
  const body = JSON.stringify(obj);
  res.writeHead(code, { 'Content-Type': 'application/json; charset=utf-8', 'Cache-Control': 'no-store' });
  res.end(body);
}

function readBody(req) {
  return new Promise((resolve) => {
    let buf = '';
    req.on('data', (c) => { buf += c; });
    req.on('end', () => { try { resolve(JSON.parse(buf || '{}')); } catch (e) { resolve({}); } });
  });
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, 'http://127.0.0.1');
  const p = url.pathname;

  try {
    if (p === '/' || p === '/index.html') {
      const html = fs.readFileSync(path.join(PUBLIC_DIR, 'index.html'), 'utf8');
      res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store' });
      return res.end(html);
    }

    if (p === '/api/state') {
      const agents = await buildAgents();
      const wins = await getWindows();
      const procMap = await getProcMap();
      return sendJson(res, 200, {
        ok: true,
        serverTime: new Date().toISOString(),
        today: isoDate(0),
        agents,
        tasks: state.tasks,
        dsh: readDshPlugins(),
        dshCmd: 'dsh plugin --profile web add <包名>',
        bridge: {
          processProbe: Object.keys(procMap).length ? 'tasklist' : 'unavailable',
          windowProbe: wins.length ? 'powershell' : 'blocked',
          windowCount: wins.length
        }
      });
    }

    if (p === '/api/windows') {
      const wins = await getWindows(true);
      const procMap = await getProcMap();
      const procs = Object.keys(procMap).map((name) => ({ name: name, pids: procMap[name] }))
        .sort((a, b) => b.pids.length - a.pids.length).slice(0, 60);
      return sendJson(res, 200, { ok: true, windows: wins, procs: procs });
    }

    if (p === '/api/focus' && req.method === 'POST') {
      const body = await readBody(req);
      const id = body.id;
      const cfg = readJson(AGENTS_FILE, { agents: [] });
      const a = (cfg.agents || []).find((x) => x.id === id);
      if (!a) return sendJson(res, 404, { ok: false, reason: 'unknown_agent' });

      const agents = await buildAgents();
      const live = agents.find((x) => x.id === id);
      const pid = body.pid || (live ? live.pid : 0);

      if (!pid && !a.titleHint) return sendJson(res, 200, { ok: false, reason: 'no_target' });

      // 派发模式：先把任务文本放进系统剪贴板，再置顶窗口
      if (body.text) await runWin('clip', ['-Text', String(body.text)]);

      const r = await runWin('focus', pid ? ['-ProcId', String(pid)] : ['-Title', a.titleHint]);
      return sendJson(res, 200, r || { ok: false, reason: 'bridge_failed' });
    }

    if (p === '/api/run' && req.method === 'POST') {
      const body = await readBody(req);
      const cfg = readJson(AGENTS_FILE, { agents: [] });
      const a = (cfg.agents || []).find((x) => x.id === body.id);
      if (!a || !a.launch || !a.launch.cmd) return sendJson(res, 404, { ok: false, reason: 'unknown_agent' });
      if (!a.dispatch || a.dispatch.mode !== 'cli') return sendJson(res, 200, { ok: false, reason: 'no_cli' });
      const text = String(body.text || '').replace(/["\r\n]/g, ' ').trim().slice(0, 800);
      if (!text) return sendJson(res, 200, { ok: false, reason: 'empty_text' });
      const base = (a.dispatch && a.dispatch.cmd) || (a.launch ? a.launch.cmd + ' run' : null);
      if (!base) return sendJson(res, 200, { ok: false, reason: 'no_cmd' });
      try {
        const child = spawn('cmd.exe', ['/k', base, '"' + text + '"'], {
          cwd: a.dir || process.cwd(), detached: true, stdio: 'ignore', windowsHide: false
        });
        child.unref();
        return sendJson(res, 200, { ok: true, dispatched: true, text: text });
      } catch (e) {
        return sendJson(res, 200, { ok: false, reason: 'spawn_failed', detail: String(e.message) });
      }
    }

    if (p === '/api/launch' && req.method === 'POST') {
      const body = await readBody(req);
      const cfg = readJson(AGENTS_FILE, { agents: [] });
      const a = (cfg.agents || []).find((x) => x.id === body.id);
      if (!a || !a.launch) return sendJson(res, 404, { ok: false, reason: 'unknown_agent' });

      try {
        const child = spawn(a.launch.cmd, a.launch.args || [], {
          cwd: a.dir || process.cwd(),
          detached: true,
          shell: true,
          stdio: 'ignore',
          windowsHide: false
        });
        child.unref();
        return sendJson(res, 200, { ok: true, launched: true, url: a.url || '' });
      } catch (e) {
        return sendJson(res, 200, { ok: false, reason: 'spawn_failed', detail: String(e.message) });
      }
    }

    if (p === '/api/bind' && req.method === 'POST') {
      const body = await readBody(req);
      if (!state.bindings) state.bindings = {};
      if (body.pid) state.bindings[body.id] = Number(body.pid);
      else delete state.bindings[body.id];
      saveState();
      return sendJson(res, 200, { ok: true, bindings: state.bindings });
    }

    if (p === '/api/task' && req.method === 'POST') {
      const body = await readBody(req);
      const act = body.action;
      if (act === 'add') {
        state.tasks.push({
          id: 't' + Date.now().toString(36),
          title: String(body.title || '').trim() || '未命名任务',
          agent: body.agent || 'workbuddy',
          due: body.due || isoDate(0),
          status: 'todo',
          note: body.note || ''
        });
      } else if (act === 'update') {
        const t = state.tasks.find((x) => x.id === body.id);
        if (t) {
          if (body.title !== undefined) t.title = body.title;
          if (body.agent !== undefined) t.agent = body.agent;
          if (body.due !== undefined) t.due = body.due;
          if (body.status !== undefined) t.status = body.status;
          if (body.note !== undefined) t.note = body.note;
        }
      } else if (act === 'delete') {
        state.tasks = state.tasks.filter((x) => x.id !== body.id);
      } else if (act === 'clearSample') {
        state.tasks = [];
        state.cleared = true;
      }
      saveState();
      return sendJson(res, 200, { ok: true, tasks: state.tasks });
    }

    if (p === '/api/backup' && req.method === 'GET') {
      res.writeHead(200, {
        'Content-Type': 'application/json; charset=utf-8',
        'Content-Disposition': 'attachment; filename="agent-console-backup-' + isoDate(0) + '.json"'
      });
      return res.end(JSON.stringify({ version: 1, exportedAt: new Date().toISOString(), tasks: state.tasks, bindings: state.bindings }, null, 2));
    }

    if (p === '/api/restore' && req.method === 'POST') {
      const body = await readBody(req);
      if (Array.isArray(body.tasks)) {
        state.tasks = body.tasks;
        state.bindings = body.bindings || {};
        saveState();
        return sendJson(res, 200, { ok: true, count: state.tasks.length });
      }
      return sendJson(res, 400, { ok: false, reason: 'bad_payload' });
    }

    return sendJson(res, 404, { ok: false, reason: 'not_found' });
  } catch (e) {
    return sendJson(res, 500, { ok: false, reason: 'server_error', detail: String(e.message) });
  }
});

server.listen(PORT, '127.0.0.1', () => {
  console.log('agent-console 已启动: http://127.0.0.1:' + PORT);
  console.log('按 Ctrl+C 停止');
});
