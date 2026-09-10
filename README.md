# 智能体指挥台（agent-console）

把本机的 WorkBuddy / OpenCode / DeepSeek Harness 当成员工来管的 Web 工作台。

## 启动（2026-09-08 更新：已改为双模式）

- **本地模式（推荐日常用）**：直接双击 `public\index.html`。
  任务增删改查照常可用，数据存在浏览器 localStorage（换浏览器/清缓存会丢，请勤用「导出 JSON」）。
  智能体状态探测、窗口置顶、远程启动这些浏览器做不到的能力**停用**，页面会显示「本地模式」。
  原一键启动器已移除（2026-09-08 用户决定暂时放弃该功能）。
- **桥接模式（可选）**：`node server.js 8765` 后访问 http://127.0.0.1:8765，
  恢复全部能力（探测/置顶/启动，任务存 state.json）。

## 为什么需要一个 exe 常驻

浏览器里的 HTML **无权操纵操作系统窗口**，这是浏览器的安全底线。
所以「唤起 + 置顶」必须经由本机桥接：

```
浏览器 index.html  ──fetch──>  server.js (Node，零依赖)
                                  │
                                  ├─ tasklist      探测进程（判断是否在运行）
                                  ├─ net.Socket    探测端口
                                  └─ win.ps1       唤起并置顶窗口
```

## 置顶的三级降级

| 级别 | 手段 | 说明 |
|---|---|---|
| 1 | COM `WScript.Shell.AppActivate(pid)` | 轻量，能还原最小化窗口 |
| 2 | P/Invoke `SetForegroundWindow` | 最强，直接操作窗口句柄 |
| 3 | 浏览器 `window.open(url).focus()` | 仅 web 类（dsh），不依赖系统权限 |

前两级依赖 PowerShell。若被安全策略拦截，页面顶部会显示
「窗口探测被拦截 · 按 PID 置顶」，此时改用「绑定」按钮手动指定 PID，置顶依然可用。

> 注意：WorkBuddy 自身的沙箱会拦截 AI 调用 PowerShell 的 COM / Add-Type，
> 所以我在开发环境里**无法验证**第 1、2 级。你双击 bat 启动后点一次「唤起置顶」即可确认。

## 四个智能体（实测配置）

| id | 定位 | 探测方式 | 备注 |
|---|---|---|---|
| workbuddy | 指挥台 / 编排者 | 进程 `WorkBuddy.exe` | 唯一编排者，别让它降级成普通 worker |
| opencode | 开发台（TUI） | 进程 `OpenCode.exe` | 可被脚本调度：`opencode run "任务" --agent X --dir D --format json` |
| opencode-acp | 调度接口 | 端口 8899 | Agent Client Protocol，跨客户端派活的正规通道 |
| dsh | 能力扩展台 | 端口 3080 + `~/.dsh/dsh-process.json` | 插件化运行时，能力边界由插件决定 |

**dsh 的真实端口是 3080，不是 8080**（实测 `netstat` 确认）。它的 PID 会写进
`~/.dsh/dsh-process.json`，服务据此探测，不依赖端口。

## 增删智能体

改 `agents.json`：

```json
{
  "id": "myagent",
  "name": "My Agent",
  "subtitle": "测试台",
  "role": "干什么用的",
  "kind": "app | tui | service | web",
  "color": "#2563eb",
  "dir": "C:\\工作目录",
  "launch": { "cmd": "myagent", "args": [] },
  "url": "",                                   // web 类填地址
  "detect": { "process": "", "port": 0, "pidFile": "" },
  "titleHint": "窗口标题关键词",
  "strength": "它擅长什么",
  "caveat": "它的短板"
}
```

刷新页面即生效，无需重启服务。

## dsh 装插件

```bash
dsh plugin --profile web add <包名>
```

装完重启 `dsh --profile web` 生效。当前已装 13 个插件包，页面「插件舱」区可见。

## 数据安全

- 任务数据：桥接模式存 `state.json`；本地模式存浏览器 localStorage（导出 JSON 备份）
- 清空需二次确认
- 桥接模式只监听 `127.0.0.1`，外部机器访问不到

## 分工纪律（重要）

1. **worker 之间零直接通信**。所有派发经过你或 WorkBuddy，避免循环调用和上下文污染
2. **输出必须结构化**：任务ID / 状态 / 结论≤3行 / 证据与数据源 / 需要你决策的问题
3. **知识外置**：交易铁律、席位身份表这类知识做成文件让 agent 按需读取，不要塞进 system prompt
