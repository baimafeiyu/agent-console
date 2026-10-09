# 智能体指挥台 · agent-console

一个 Windows 桌面工作台：把本机的 AI 工具（WorkBuddy / OpenCode / DeepSeek Harness / ChatGPT 等）
当"员工"管理 —— **状态实时监测 → 一键唤起置顶 → 任务派发**，外加 **用量/余额监测**、**鲸鱼娘桌面宠物**，
以及把差旅费报销自动化（技能 `oa-reimburse-draft`）内嵌成左侧第五栏的 **报销操作台**。

技术栈：**Tauri v2（Rust + WebView2）**，内置纯 Rust HTTP 服务，前端为原生 HTML/CSS/JS（无构建步骤）。
运行内存约 40 MB，可执行文件约 6 MB，托盘常驻。

---

## 功能一览

### 主工作台（main 窗口，1280×840）
- **5 个智能体状态探测**：进程名 / 端口 / 窗口标题 / PID 文件四路探测，实时显示在线状态
- **一键唤起并置顶**：Win32 `EnumWindows + SetForegroundWindow`（含 Alt 键绕前台锁），非 PowerShell 桥接
- **任务派发**：
  - OpenCode → 真执行（`opencode run "任务"`，弹独立控制台窗口可见输出）
  - DeepSeek Harness → 真执行（`dsh --profile headless "任务"`）
  - WorkBuddy / ChatGPT → 剪贴板派发（任务文本入剪贴板 + 窗口置顶，Ctrl+V 回车下发）
- 任务 CRUD / 导入导出 / 清空（`state.json`）
- **OpenCode Go 用量面板**：5 小时 / 本周 / 本月三档进度条，60s 自动刷新
- 顶栏 ✦ 按钮：显隐鲸鱼娘宠物

### 报销操作台（main 窗口左侧第五栏）
把技能 `oa-reimburse-draft` 的网页操作台**移植进主窗口**：不另开浏览器、不另起页面，
在指挥台里把一张差旅费报销单从「选票据」做到「保存草稿 + 复核卡」。

- **页面**：`public/index.html` 的 `[data-section="reimburse"]` 段（外观映射到本应用 token，浅色/深色跟随）
- **干活的仍是技能自己**：页面只做交互，真正执行的是技能自带的 `web/server.mjs` + `runner/`
- **接线**：Rust 只做两件事 —— 起/停 `web/server.mjs`，并把 `/api/reimburse/*` **反向代理**给它
  （对页面同源 `127.0.0.1:8766`，因此**不必**给 `server.mjs` 加 CORS）
- **实时日志走轮询**（`/api/reimburse/run-poll`），不是 SSE：页面经 Rust 反代访问技能，而
  **tiny_http 对 chunked 流式响应会攒着不发** —— `agent-prefill` 跑 27s，SSE 的响应头也在 27s 才到，
  页面等于全程看不到进度（早先「81 个事件 1.6s 分散到达」的实测跑得太短，区分不出「流式」与
  「攒完一次吐」）。直连 8790 时 SSE 是好的（33ms 连上、事件分散 35.8s 到达），所以 SSE 留给技能自带操作台页面
- **第二步 = AI 预处理**（`agent-prefill`）：一键扫批次目录 → **规则先归类**（快/免费/确定）→
  只把**拿不准的**附件渲染成图交给视觉模型（`deepseek-v4-flash-vision-exp`）判该进哪个文件夹 →
  同时出**工单草案**。跑完把草案回填表单，并渲染「AI 决策」审查表（逐附件判归与依据 / AI 改判数 / 模型名 / 待核假设）
  - 慢（1–3 分钟），进度看右侧「运行日志」；**确认或微调后**照常「生成工单」→「开始执行」
  - 用户 2026-10-08 明示：部门简称**固定「临海」**、事由**给建议即可**、**分摊部门规则省略**（走系统默认的经办人部门）
- **备选：只读补助表预填**（`/api/prefill`）：目录里有「伙食补贴申请表」xlsx 时，
  点一下把 **出差人 / 事由 / 伙食费** 读出来（这三项是**事实**，技能本来就会算），每个值都标**出处**
  - **不允许预填的**：报销说明措辞、收款人/开户行/尾号、rowKey
    —— 措辞是判断（技能决策记录 #4：工具不猜部门、不猜职级），收款信息受 K2 人工复核约束
  - 读不出就**照实说原因**：没表 / 表是图片 / 多张表 / 读失败 / 结构不符，五种各有各的话，绝不猜
- **技能红线原样保留**：只填不提交（页面无「提交」入口）、金额一律不碰、写入白名单 4 列
  —— 全部由 `server.mjs` 的命令白名单与路径限制强制，本应用不复制这套逻辑
- **外观与版式**：面板 / 贴纸 / 图标徽章 / 表单分组全部复用主界面的设计语言；顶部一条 ①→④ **流程轨**
  （点一步跳到那一步）：① 选票据目录 ② AI 预处理 ③ 确认工单 ④ 执行建单。
  跑完的「运行结果与复核」是**无编号**面板（自动出现，不是要人做的一步）
  （版式照抄主界面《今日指挥舱》：主内容列 + 吸顶侧栏）
  - ①→④ **全部在一列里自上而下**，编号序列绝不跨栏；右侧只放**没有编号**的
    运行日志与踩坑速查（所以能 sticky 常驻，点「开始执行」时日志就在旁边）
  - 运行日志做成深色终端，与浅色表单拉开层次。底色走专门的 `--terminal` token ——
    **别拿 `--ink-deep` 当背景**，它在深色主题下会翻成近白色。插件舱的 `.code-bar` 原来就是这么写的，
    深色主题下白底白字，本次一并修掉
  - 窄屏（≤560px）两列/三列自动摊成一列；`min-width:0` 必须留着，否则长下拉会把整页撑宽

### 鲸鱼娘桌面宠物（pet / petmenu / net 三个透明窗）
- 素材：`whale-refined` 雪碧图（1536×1872 = 8 列 × 9 行，cell 192×208）
- 动画轨道：idle / running-right / running-left / waving / jumping / failed / waiting / running / review
- **拖拽**：Rust 自研（`GetCursorPos` + `GetAsyncKeyState` 跟手轮询），不使用 WebView2 原生拖拽
- **左键互动三通道**：带动作专属台词；闲置自动播闲置动作
- **右键菜单**：隐藏宠物 / 查余额（Go 用量、DeepSeek 余额）/ 打开工作台 / 唤起各智能体
- **网速气泡**（net 窗）：`GetIfTable2` 采样实时上下行（独立小窗，可菜单开关，配置 `net-bubble.json` 改完 2 秒生效）
- **视觉**：动漫化椭圆气泡（实测色板 + 线形规范，见 `气泡视觉规范-动漫化.md`）
- 动作/行为**清单驱动**（`pet-behavior.json`），换动作改图不需要重编译

### 开发工具（devtools）
- `pet-engine-test.cjs` 宠物动作引擎离线测试（DOM 桩）
- `net-fmt-test.cjs` 网速显示口径离线测试
- `preview.cjs` 无头预览（生成 `preview-*.png`，用于免启动看气泡样式）
- `rc-ui-smoke.mjs` 报销操作台栏目端到端冒烟（导航 / 反代 / 目录浏览 / 红线守卫 /
  **步骤顺序不许跨栏** / **不存在第 5 步** / 深色主题终端 / 窄屏适配 /
  **补助表预填 + 预填不许碰红线字段** / 老栏目回归），共 41 项断言；
  加 `--deep` 再多跑一节**真调视觉模型的 AI 预处理端到端**（草案面板 / 出差人 / 事由 /
  明细定位 / 部门简称 / 批次号），47 项断言，约 1–3 分钟
  —— 需应用正在运行；借用技能目录里的 playwright（路径由 `reimburse.json` 的 `skillRoot` 推出）

---

## 架构

```
agent-console.exe (Tauri v2)
├─ Rust HTTP 服务 (tiny_http, 127.0.0.1:8766, 4 worker)
│   ├─ 静态: /index.html  /pet.html  /menu.html  /net.html  /spritesheet.webp
│   └─ API:  /api/state  /api/focus  /api/launch  /api/run  /api/bind  /api/task
│            /api/backup  /api/restore  /api/windows  /api/usage  /api/dshbalance
│            /api/openurl  /api/pet/toggle  /api/pet/drag  /api/pet/geom
│            /api/main/show  /api/quit
│            /api/reimburse/service   报销操作台服务 起/停/查
│            /api/reimburse/*         → 反代 127.0.0.1:8790（含 SSE 流式日志）
├─ 窗口（前端全部经 http://127.0.0.1:8766/*.html 加载，运行时读盘 + no-store）
│   main    1280×840   普通窗口
│   pet     204×280    transparent / alwaysOnTop / skipTaskbar / resizable:false / focus:false / decorations:false
│   petmenu 200×310    同 pet + visible:false（右键才显示）
│   net     166×56     同 pet + visible:false，**运行时按需创建**（不在 tauri.conf.json）
├─ 托盘：显示指挥台 / 退出；关窗 = 隐藏到托盘
└─ AppHandle → OnceLock 桥接给 HTTP 线程（窗口直控端点用）
```

> 三个透明窗统一带 `additionalBrowserArgs = "--disable-gpu --disable-features=RendererCodeIntegrity"`。

---

## 目录结构

```
agent-console/
├─ desktop/
│  ├─ make-icon.js                    图标生成脚本（node 跑一次）
│  └─ src-tauri/
│     ├─ Cargo.toml                   tauri2(tray-icon) / tiny_http / ureq / arboard / windows-sys
│     ├─ tauri.conf.json              窗口声明（main + pet + petmenu；net 不在此）
│     ├─ capabilities/default.json    ⚠️ 窗口权限白名单（缺它所有 JS 窗口 API 静默失败）
│     ├─ assets/spritesheet.webp      编译期内嵌兜底素材
│     ├─ icons/icon.ico
│     └─ src/
│        ├─ main.rs                   tauri::Builder / setup / 托盘 / 宠物初始定位
│        └─ server.rs                 HTTP 服务 + 探测 / 置顶 / 派发 / 余额 / 拖拽
├─ public/                            前端（运行时从磁盘读）
│  ├─ index.html                      主工作台
│  ├─ pet.html  menu.html  net.html   宠物 / 右键菜单 / 网速气泡
│  └─ pet.json  pet-behavior.json  spritesheet.webp
├─ devtools/                          离线测试与预览工具
├─ agents.json                        智能体配置（增删智能体只改这里）
├─ net-bubble.json                    网速气泡配置（enable / gapLp）
├─ reimburse.json                     报销操作台接线（skillRoot / port / node）
├─ state.json                         任务数据
├─ restart.cmd                        优雅重启（quit → 等端口释放 → 拉起）
├─ server.js / win.ps1                旧的 Node 桥接形态（保留，桌面版不依赖）
└─ *.md                               交接与设计文档（见下方索引）
```

---

## 构建与运行

### 前置
- Rust stable-msvc（cargo）+ VS Build Tools（VC++ workload）+ WebView2 Runtime
- Node 仅用于图标生成与 devtools 脚本

### 构建
```powershell
# ⚠️ 编译前必须先退出正在运行的应用（exe 被锁会报 os error 5）
Get-Process agent-console -ErrorAction SilentlyContinue | Stop-Process -Force

cd desktop\src-tauri
$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path
cargo build --release
Start-Process target\release\agent-console.exe
```

Git Bash 环境需前置 MSVC（否则 `/usr/bin/link.exe` 抢占链接器，报 `link: extra operand`）：
```bash
export PATH="/c/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/VC/Tools/MSVC/14.44.35207/bin/Hostx64/x64:$HOME/.cargo/bin:$PATH"
export LIB='C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\14.44.35207\lib\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\um\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\ucrt\x64'
```

### 停止（优先优雅退出）
```powershell
curl -X POST http://127.0.0.1:8766/api/quit
```

---

## 配置文件速查

| 文件 | 作用 | 生效方式 |
|---|---|---|
| `agents.json` | 智能体清单：探测方式 / 启动命令 / 派发模式 / 文案 | 刷新页面 |
| `public/pet.json` | 宠物雪碧图清单（帧数/时长/轨道） | 2 秒热重载 |
| `public/pet-behavior.json` | 动作与行为清单（左键三通道、闲置动作、台词） | 2 秒热重载 |
| `net-bubble.json` | 网速气泡开关与间距 | 2 秒热重载 |
| `reimburse.json` | 报销操作台接线：`skillRoot`（技能目录）/ `port`（默认 8790）/ `node`（留空用 PATH） | 每次调用回读，即时生效 |

---

## 文档索引

| 文档 | 内容 |
|---|---|
| `交接文档-桌面版指挥台.md` | **总体交接**：架构 / 功能 / 实测事实 / 构建 / 风险 / 路线 |
| `启动卡死-交接文档.md` | 启动卡死专题：症状判别 / 已排除项 / **根因（WorkBuddy 沙箱）** / 操作红线 |
| `交接-任务日志与需求清单.md` | 项目全过程日志（需求清单 + 时间线） |
| `宠物动作制作指南.md` | 加动作/换图三条路、图集硬规格、9 行动作表 |
| `气泡视觉规范-动漫化.md` | 气泡色板/线形规范、椭圆度取舍、宽度公式 |
| `网速气泡-设计方案.md` | 网速数据源实测、网卡过滤口径、`GetIfTable2` 实现 |

---

## 已知问题

1. **启动卡死在 WorkBuddy 沙箱内**（进程活、HTTP 正常、`setup()` 不执行）
   - 根因已定位：**只在 WorkBuddy 命令沙箱内启动时出现**，与代码/WebView2 运行时无关
   - 规避：**用资源管理器双击启动**，或将 `agent-console.exe` 加入沙箱白名单
   - 详见 `启动卡死-交接文档.md`
2. 宠物位置不落盘，重启回默认位（右下角）
3. `/api/usage`、`/api/dshbalance` 服务端 60s 缓存
4. 派发的执行窗口用 `cmd /k` 常驻，跑完需手动关闭

## ⛔ 两条红线（踩过的坑，改动前必读）

1. **不要给 `transparent` 窗口加 `resizable:true`，也不要在运行时对它调 `set_size()`** —— 会导致渲染进程崩溃（有崩溃转储佐证）
2. **不要把 `net` 窗口写进 `tauri.conf.json`** —— 声明进配置会让启动卡在创建它；必须保持运行时按需创建（`ensure_net_window()`），这样即使建窗失败 `setup()` 也能跑完

---

## 隐私说明

- 所有 API key 存放于 `~/.dsh/.credentials.yaml`，应用**运行时读取，绝不写入日志或提交**
- HTTP 服务仅监听 `127.0.0.1`
- `.gitignore` 已排除构建产物（`target/` ~3.4GB）与运行日志
