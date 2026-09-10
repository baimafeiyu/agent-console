use arboard::Clipboard;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Read;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tiny_http::{Header, Method, Response, Server};

pub fn project_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap()
        .to_path_buf()
}

fn home_dir() -> PathBuf {
    std::env::var("USERPROFILE").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
}

fn read_json(path: &PathBuf, fallback: Value) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(fallback)
}

fn write_json(path: &PathBuf, v: &Value) {
    let s = serde_json::to_string_pretty(v).unwrap_or_default();
    let _ = std::fs::write(path, s);
}

fn load_state() -> Value {
    let f = project_dir().join("state.json");
    let st = read_json(&f, Value::Null);
    let valid = st.is_object()
        && st.get("tasks").map(|t| t.is_array()).unwrap_or(false);
    if valid {
        return st;
    }
    let today = iso_date(0);
    let st = json!({
        "tasks": [
            { "id": "t1", "title": "核对台州烟草空调维护报价（2027-2029）三档单价", "agent": "workbuddy", "due": iso_date(-2), "status": "todo", "note": "跨期混合报价，必须拆到每档单独核算，禁止正则解析" },
            { "id": "t2", "title": "smart-reimburse 水费填报模板适配宜搭上传规范", "agent": "opencode", "due": today.clone(), "status": "doing", "note": "xls 列序需与平台模板一致" },
            { "id": "t3", "title": "给 dsh 装一个 PDF 解析插件", "agent": "dsh", "due": today.clone(), "status": "todo", "note": "dsh plugin --profile web add <包名>" },
            { "id": "t4", "title": "复盘：半导体材料主线板块地位判定", "agent": "workbuddy", "due": iso_date(1), "status": "todo", "note": "先画板块时间线，再定龙头/补涨" },
            { "id": "t5", "title": "验证 ACP 端口 8899 能否被外部客户端驱动", "agent": "opencode-acp", "due": iso_date(3), "status": "todo", "note": "" }
        ],
        "bindings": {},
        "cleared": false
    });
    write_json(&f, &st);
    st
}

fn iso_date(offset_days: i64) -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
        + offset_days * 86400;
    let days = secs.div_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/* ---------------- 探测层 ---------------- */

fn run_tasklist() -> BTreeMap<String, Vec<u32>> {
    let mut map: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    let mut cmd = Command::new("tasklist");
    cmd.args(["/FO", "CSV", "/NH"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW —— 不闪黑窗
    }
    if let Ok(out) = cmd.output() {
        let s = String::from_utf8_lossy(&out.stdout);
        for line in s.lines() {
            let fields: Vec<&str> = parse_csv_line(line);
            if fields.len() < 2 { continue; }
            if let Ok(pid) = fields[1].trim().parse::<u32>() {
                map.entry(fields[0].to_lowercase()).or_default().push(pid);
            }
        }
    }
    map
}

fn parse_csv_line(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut in_q = false;
    let mut start = 0usize;
    let b = line.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'"' => in_q = !in_q,
            b',' if !in_q => {
                out.push(line[start..i].trim_matches('"'));
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < line.len() { out.push(line[start..].trim_matches('"')); }
    out
}

fn pid_alive(procs: &BTreeMap<String, Vec<u32>>, pid: u32) -> bool {
    procs.values().any(|v| v.contains(&pid))
}

fn check_port(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(800)).is_ok()
}

/* ---------------- Win32 窗口层 ---------------- */

#[cfg(windows)]
mod win32 {
    use windows_sys::Win32::Foundation::{HWND, LPARAM, BOOL};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextW, IsWindowVisible, IsIconic, SetForegroundWindow,
        ShowWindow, BringWindowToTop, GetWindowThreadProcessId, SW_RESTORE,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP, VK_MENU};

    pub struct WinInfo {
        pub hwnd: isize,
        pub pid: u32,
        pub title: String,
    }

    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = &mut *(lparam as *mut Vec<WinInfo>);
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), 512) as usize;
        if n == 0 {
            return 1;
        }
        let title = String::from_utf16_lossy(&buf[..n]);
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        list.push(WinInfo { hwnd: hwnd as isize, pid, title });
        1
    }

    pub fn list_windows() -> Vec<WinInfo> {
        let mut list: Vec<WinInfo> = Vec::new();
        unsafe {
            EnumWindows(Some(enum_cb), &mut list as *mut Vec<WinInfo> as LPARAM);
        }
        list
    }

    /// 返回 Ok(hwnd) 或 Err(reason)
    pub fn focus(target_pid: u32, title: &str) -> Result<isize, &'static str> {
        let wins = list_windows();
        let mut hwnd: isize = 0;
        if target_pid > 0 {
            if let Some(w) = wins.iter().find(|w| w.pid == target_pid) {
                hwnd = w.hwnd;
            }
        }
        if hwnd == 0 && !title.is_empty() {
            let t = title.to_lowercase();
            if let Some(w) = wins.iter().find(|w| w.title.to_lowercase().contains(&t)) {
                hwnd = w.hwnd;
            }
        }
        if hwnd == 0 {
            return Err("no_window");
        }
        unsafe {
            let h = hwnd as HWND;
            if IsIconic(h) != 0 {
                ShowWindow(h, SW_RESTORE);
            }
            // 先按一下 Alt，绕过前台锁定策略
            keybd_event(VK_MENU as u8, 0, 0, 0);
            BringWindowToTop(h);
            let ok = SetForegroundWindow(h);
            keybd_event(VK_MENU as u8, 0, KEYEVENTF_KEYUP, 0);
            if ok == 0 {
                return Err("foreground_denied");
            }
        }
        Ok(hwnd)
    }
}

#[cfg(windows)]
use win32::{focus as win_focus, list_windows};

#[cfg(not(windows))]
fn list_windows() -> Vec<()> { Vec::new() }

#[cfg(not(windows))]
fn win_focus(_pid: u32, _title: &str) -> Result<isize, &'static str> { Err("unsupported") }

fn set_clipboard(text: &str) -> bool {
    match Clipboard::new() {
        Ok(mut c) => c.set_text(text.to_string()).is_ok(),
        Err(_) => false,
    }
}

/* ---------------- OpenCode Go 用量 ---------------- */

fn cred_key(name: &str) -> Option<String> {
    let creds = std::fs::read_to_string(home_dir().join(".dsh").join(".credentials.yaml")).ok()?;
    for line in creds.lines() {
        if let Some(rest) = line.trim_start().strip_prefix(&format!("{}:", name)) {
            let v = rest.trim().trim_matches('"').trim_matches('\'').to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn go_key() -> Option<String> {
    cred_key("OPENCODE_GO_API_KEY")
}

static USAGE_CACHE: std::sync::OnceLock<Mutex<Option<(std::time::Instant, Value)>>> =
    std::sync::OnceLock::new();

fn fetch_usage() -> Value {
    let cache = USAGE_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(c) = cache.lock() {
        if let Some((at, v)) = c.as_ref() {
            if at.elapsed() < Duration::from_secs(60) {
                return v.clone();
            }
        }
    }
    let key = match go_key() {
        Some(k) => k,
        None => return json!({ "ok": false, "reason": "no_key" }),
    };
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).build();
    let resp = agent
        .get("https://opencode.ai/zen/go/v1/usage")
        .set("Authorization", &format!("Bearer {}", key))
        .call();
    let out = match resp {
        Ok(r) => {
            let mut s = String::new();
            let _ = r.into_reader().read_to_string(&mut s);
            let v: Value = serde_json::from_str(&s).unwrap_or(json!({}));
            match v.get("usage") {
                Some(usage) => json!({ "ok": true, "usage": usage }),
                None => json!({ "ok": false, "reason": "invalid", "detail": s.chars().take(200).collect::<String>() }),
            }
        }
        Err(e) => json!({ "ok": false, "reason": "network", "detail": e.to_string() }),
    };
    if let Ok(mut c) = cache.lock() {
        *c = Some((std::time::Instant::now(), out.clone()));
    }
    out
}

/* ---------------- 状态聚合 ---------------- */

fn build_agents(shared: &Arc<Mutex<Value>>) -> Value {
    let cfg = read_json(&project_dir().join("agents.json"), json!({"agents": []}));
    let agents = cfg.get("agents").and_then(|a| a.as_array()).cloned().unwrap_or_default();
    let procs = run_tasklist();
    let wins = list_windows();

    let state = shared.lock().unwrap();
    let empty = Vec::new();
    let tasks = state.get("tasks").and_then(|t| t.as_array()).unwrap_or(&empty);
    let bindings = state.get("bindings").and_then(|b| b.as_object()).cloned().unwrap_or_default();

    let list: Vec<Value> = agents.iter().map(|a| {
        let det = a.get("detect").cloned().unwrap_or(json!({}));
        let port = det.get("port").and_then(|p| p.as_u64()).unwrap_or(0) as u16;
        let port_up = port > 0 && check_port(port);

        let pname = det.get("process").and_then(|p| p.as_str()).unwrap_or("");
        let proc_hits: Vec<u32> = if pname.is_empty() { Vec::new() } else {
            procs.get(&pname.to_lowercase()).cloned().unwrap_or_default()
        };

        let mut pidfile_pid = 0u32;
        if let Some(pf) = det.get("pidFile").and_then(|p| p.as_str()) {
            let path = if let Some(rest) = pf.strip_prefix("~/") {
                home_dir().join(rest)
            } else {
                PathBuf::from(pf)
            };
            if let Some(p) = read_json(&path, Value::Null).get("pid").and_then(|p| p.as_u64()) {
                pidfile_pid = p as u32;
            }
        }
        let pidfile_alive = pidfile_pid > 0 && pid_alive(&procs, pidfile_pid);

        let title_hint = a.get("titleHint").and_then(|t| t.as_str()).unwrap_or("");
        let title_hit = if title_hint.is_empty() { None } else {
            wins.iter().find(|w| w.title.to_lowercase().contains(&title_hint.to_lowercase()))
        };

        let running = port_up || !proc_hits.is_empty() || title_hit.is_some() || pidfile_alive;

        let id = a.get("id").and_then(|i| i.as_str()).unwrap_or("");
        let bound_pid = bindings.get(id).and_then(|p| p.as_u64()).unwrap_or(0) as u32;
        let bound_alive = bound_pid > 0 && pid_alive(&procs, bound_pid);

        let pid = if bound_alive { bound_pid }
            else if pidfile_alive { pidfile_pid }
            else if let Some(w) = title_hit { w.pid }
            else { proc_hits.first().copied().unwrap_or(0) };

        let doing: Vec<&Value> = tasks.iter().filter(|t| {
            t.get("agent").and_then(|x| x.as_str()) == Some(id)
                && t.get("status").and_then(|x| x.as_str()) == Some("doing")
        }).collect();
        let todo: Vec<&Value> = tasks.iter().filter(|t| {
            t.get("agent").and_then(|x| x.as_str()) == Some(id)
                && t.get("status").and_then(|x| x.as_str()) == Some("todo")
        }).collect();

        json!({
            "id": id,
            "name": a.get("name"),
            "subtitle": a.get("subtitle"),
            "role": a.get("role"),
            "kind": a.get("kind"),
            "color": a.get("color"),
            "url": a.get("url").cloned().unwrap_or(json!("")),
            "strength": a.get("strength").cloned().unwrap_or(json!("")),
            "caveat": a.get("caveat").cloned().unwrap_or(json!("")),
            "titleHint": title_hint,
            "dispatch": a.get("dispatch").and_then(|d| d.get("mode")).cloned().unwrap_or(json!("clipboard")),
            "running": running,
            "portUp": port_up,
            "port": port,
            "pid": pid,
            "bound": bound_alive,
            "procCount": proc_hits.len(),
            "windowTitle": title_hit.map(|w| w.title.clone()).unwrap_or_default(),
            "doingCount": doing.len(),
            "todoCount": todo.len(),
            "currentTask": doing.first()
                .and_then(|t| t.get("title")).cloned()
                .or_else(|| todo.first().and_then(|t| t.get("title")).cloned())
                .unwrap_or(json!(""))
        })
    }).collect();
    Value::Array(list)
}

fn dsh_info() -> Value {
    let pkg = read_json(&home_dir().join(".dsh").join("profiles").join("web").join("package.json"), Value::Null);
    match pkg.get("dsh").and_then(|d| d.get("profile")).and_then(|p| p.get("bundles")).cloned() {
        Some(bundles) if bundles.is_array() => json!({ "bundles": bundles, "available": true }),
        _ => json!({ "bundles": [], "available": false }),
    }
}

/* ---------------- API 处理 ---------------- */

fn json_response(mut req: tiny_http::Request, code: u32, body: Value) {
    let header = Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap();
    let header2 = Header::from_bytes("Cache-Control", "no-store").unwrap();
    let resp = Response::from_string(body.to_string())
        .with_status_code(code)
        .with_header(header)
        .with_header(header2);
    let _ = req.respond(resp);
}

fn find_agent(id: &str) -> Option<Value> {
    let cfg = read_json(&project_dir().join("agents.json"), json!({"agents": []}));
    cfg.get("agents")?.as_array()?.iter()
        .find(|a| a.get("id").and_then(|i| i.as_str()) == Some(id))
        .cloned()
}

fn handle_api(shared: &Arc<Mutex<Value>>, path: &str, body: &Value) -> (u32, Value) {
    match path {
        "/api/state" => {
            let agents = build_agents(shared);
            let wins = list_windows();
            let state = shared.lock().unwrap();
            (
                200,
                json!({
                    "ok": true,
                    "serverTime": iso_date(0),
                    "today": iso_date(0),
                    "agents": agents,
                    "tasks": state.get("tasks").cloned().unwrap_or(json!([])),
                    "dsh": dsh_info(),
                    "dshCmd": "dsh plugin --profile web add <包名>",
                    "bridge": {
                        "processProbe": "tasklist",
                        "windowProbe": if wins.is_empty() { "blocked" } else { "powershell" },
                        "windowCount": wins.len()
                    },
                    "runtime": "tauri"
                }),
            )
        }
        "/api/windows" => {
            let wins: Vec<Value> = list_windows().iter().map(|w| json!({
                "pid": w.pid, "name": "", "title": w.title
            })).collect();
            let procs = run_tasklist();
            let mut list: Vec<(String, Vec<u32>)> = procs.into_iter().collect();
            list.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
            let procs_json: Vec<Value> = list.iter().take(60)
                .map(|(name, pids)| json!({ "name": name, "pids": pids }))
                .collect();
            (200, json!({ "ok": true, "windows": wins, "procs": procs_json }))
        }
        "/api/focus" => {
            let id = body.get("id").and_then(|i| i.as_str()).unwrap_or("");
            let a = match find_agent(id) {
                Some(a) => a,
                None => return (404, json!({ "ok": false, "reason": "unknown_agent" })),
            };
            let agents = build_agents(shared);
            let live = agents.as_array().unwrap().iter()
                .find(|x| x.get("id").and_then(|i| i.as_str()) == Some(id)).cloned();
            let mut pid = body.get("pid").and_then(|p| p.as_u64()).unwrap_or(0) as u32;
            if pid == 0 {
                pid = live.as_ref().and_then(|l| l.get("pid")).and_then(|p| p.as_u64()).unwrap_or(0) as u32;
            }
            let title_hint = a.get("titleHint").and_then(|t| t.as_str()).unwrap_or("");
            if pid == 0 && title_hint.is_empty() {
                return (200, json!({ "ok": false, "reason": "no_target" }));
            }
            if let Some(text) = body.get("text").and_then(|t| t.as_str()) {
                set_clipboard(text);
            }
            match win_focus(pid, title_hint) {
                Ok(hwnd) => (200, json!({ "ok": true, "pid": pid, "hwnd": hwnd, "via": "win32" })),
                Err(reason) => (200, json!({ "ok": false, "pid": pid, "reason": reason })),
            }
        }
        "/api/launch" => {
            let id = body.get("id").and_then(|i| i.as_str()).unwrap_or("");
            let a = match find_agent(id) {
                Some(a) => a,
                None => return (404, json!({ "ok": false, "reason": "unknown_agent" })),
            };
            let cmd = a.get("launch").and_then(|l| l.get("cmd")).and_then(|c| c.as_str()).unwrap_or("");
            if cmd.is_empty() {
                return (404, json!({ "ok": false, "reason": "unknown_agent" }));
            }
            let args: Vec<String> = a.get("launch").and_then(|l| l.get("args"))
                .and_then(|x| x.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let mut line = format!("start \"\" {}", cmd);
            for arg in &args { line.push(' '); line.push_str(arg); }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let _ = Command::new("cmd").args(["/C"]).raw_arg(&line)
                    .creation_flags(0x08000000) // CREATE_NO_WINDOW
                    .spawn();
            }
            (200, json!({ "ok": true, "launched": true, "url": a.get("url").cloned().unwrap_or(json!("")) }))
        }
        "/api/run" => {
            let id = body.get("id").and_then(|i| i.as_str()).unwrap_or("");
            let a = match find_agent(id) {
                Some(a) => a,
                None => return (404, json!({ "ok": false, "reason": "unknown_agent" })),
            };
            let mode = a.get("dispatch").and_then(|d| d.get("mode")).and_then(|m| m.as_str()).unwrap_or("clipboard");
            if mode != "cli" {
                return (200, json!({ "ok": false, "reason": "no_cli" }));
            }
            let text: String = body.get("text").and_then(|t| t.as_str()).unwrap_or("")
                .chars().filter(|c| *c != '"' && *c != '\n' && *c != '\r').collect::<String>()
                .chars().take(800).collect();
            if text.is_empty() {
                return (200, json!({ "ok": false, "reason": "empty_text" }));
            }
            let base = a.get("dispatch").and_then(|d| d.get("cmd")).and_then(|c| c.as_str())
                .map(String::from)
                .unwrap_or_else(|| {
                    let lc = a.get("launch").and_then(|l| l.get("cmd")).and_then(|c| c.as_str()).unwrap_or("");
                    format!("{} run", lc)
                });
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let quoted = format!("\"{}\"", text);
                let res = Command::new("cmd").arg("/K").raw_arg(&base).raw_arg(&quoted)
                    .creation_flags(0x00000010) // CREATE_NEW_CONSOLE
                    .spawn();
                match res {
                    Ok(_) => (200, json!({ "ok": true, "dispatched": true, "text": text })),
                    Err(e) => (200, json!({ "ok": false, "reason": "spawn_failed", "detail": e.to_string() })),
                }
            }
            #[cfg(not(windows))]
            { (200, json!({ "ok": false, "reason": "unsupported" })) }
        }
        "/api/bind" => {
            let id = body.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
            let pid = body.get("pid").and_then(|p| p.as_u64()).unwrap_or(0);
            let mut state = shared.lock().unwrap();
            {
                let obj = state.as_object_mut().unwrap();
                let bindings = obj.entry("bindings").or_insert_with(|| json!({}));
                if pid > 0 {
                    bindings.as_object_mut().unwrap().insert(id, json!(pid));
                } else {
                    bindings.as_object_mut().unwrap().remove(&id);
                }
            }
            let bindings = state.get("bindings").cloned().unwrap_or(json!({}));
            write_json(&project_dir().join("state.json"), &state);
            (200, json!({ "ok": true, "bindings": bindings }))
        }
        "/api/task" => {
            let act = body.get("action").and_then(|a| a.as_str()).unwrap_or("").to_string();
            let mut state = shared.lock().unwrap();
            {
                let obj = state.as_object_mut().unwrap();
                if !obj.get("tasks").map(|t| t.is_array()).unwrap_or(false) {
                    obj.insert("tasks".to_string(), json!([]));
                }
            }
            {
                let arr = state.get_mut("tasks").unwrap().as_array_mut().unwrap();
                match act.as_str() {
                    "add" => {
                        let title = body.get("title").and_then(|t| t.as_str()).unwrap_or("").trim().to_string();
                        arr.push(json!({
                            "id": format!("t{}", now36()),
                            "title": if title.is_empty() { "未命名任务".to_string() } else { title },
                            "agent": body.get("agent").cloned().unwrap_or(json!("workbuddy")),
                            "due": body.get("due").cloned().unwrap_or(json!(iso_date(0))),
                            "status": "todo",
                            "note": body.get("note").cloned().unwrap_or(json!(""))
                        }));
                    }
                    "update" => {
                        let id = body.get("id").and_then(|i| i.as_str()).unwrap_or("");
                        for t in arr.iter_mut() {
                            if t.get("id").and_then(|i| i.as_str()) == Some(id) {
                                let obj_t = t.as_object_mut().unwrap();
                                for k in ["title", "agent", "due", "status", "note"] {
                                    if let Some(v) = body.get(k) { obj_t.insert(k.to_string(), v.clone()); }
                                }
                            }
                        }
                    }
                    "delete" => {
                        let id = body.get("id").and_then(|i| i.as_str()).unwrap_or("");
                        arr.retain(|t| t.get("id").and_then(|i| i.as_str()) != Some(id));
                    }
                    "clearSample" => arr.clear(),
                    _ => {}
                }
            }
            if act == "clearSample" {
                state.as_object_mut().unwrap().insert("cleared".to_string(), json!(true));
            }
            let tasks_out = state.get("tasks").cloned().unwrap_or(json!([]));
            write_json(&project_dir().join("state.json"), &state);
            (200, json!({ "ok": true, "tasks": tasks_out }))
        }
        "/api/usage" => (200, fetch_usage()),
        "/api/dshbalance" => (200, fetch_dshbalance()),
        "/api/pet/toggle" => (200, pet_toggle()),
        "/api/pet/drag" => (200, pet_start_drag()),
        "/api/main/show" => (200, main_show()),
        "/api/openurl" => {
            let url = body.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
            let safe = (url.starts_with("http://") || url.starts_with("https://"))
                && !url.contains(['"', '\'', '<', '>', '|', '&', '^', '%', ' ']);
            if !safe {
                return (400, json!({ "ok": false, "reason": "bad_url" }));
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let _ = Command::new("cmd").args(["/C", "start", "", &url])
                    .creation_flags(0x08000000)
                    .spawn();
            }
            (200, json!({ "ok": true }))
        }
        "/api/restore" => {
            if let Some(tasks) = body.get("tasks").and_then(|t| t.as_array()).cloned() {
                let mut state = shared.lock().unwrap();
                {
                    let obj = state.as_object_mut().unwrap();
                    obj.insert("tasks".to_string(), Value::Array(tasks));
                    obj.insert("bindings".to_string(), body.get("bindings").cloned().unwrap_or(json!({})));
                }
                let n = state.get("tasks").and_then(|t| t.as_array()).map(|a| a.len()).unwrap_or(0);
                write_json(&project_dir().join("state.json"), &state);
                (200, json!({ "ok": true, "count": n }))
            } else {
                (400, json!({ "ok": false, "reason": "bad_payload" }))
            }
        }
        _ => (404, json!({ "ok": false, "reason": "not_found" })),
    }
}

fn now36() -> String {
    format!("{:x}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis())
}

fn handle(mut req: tiny_http::Request, shared: &Arc<Mutex<Value>>) {
    let method = req.method().clone();
    let path = req.url().split('?').next().unwrap_or("/").to_string();

    let mut body = String::new();
    if method == Method::Post {
        let _ = req.as_reader().read_to_string(&mut body);
    }
    let body_json: Value = serde_json::from_str(&body).unwrap_or(json!({}));

    if method == Method::Get && (path == "/" || path == "/index.html") {
        let html_path = project_dir().join("public").join("index.html");
        match std::fs::read_to_string(&html_path) {
            Ok(html) => {
                let header = Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap();
                let header2 = Header::from_bytes("Cache-Control", "no-store").unwrap();
                let _ = req.respond(Response::from_string(html).with_header(header).with_header(header2));
                return;
            }
            Err(_) => {
                let _ = req.respond(Response::from_string("index.html not found").with_status_code(500));
                return;
            }
        }
    }

    if method == Method::Get && path == "/pet.html" {
        match std::fs::read_to_string(project_dir().join("public").join("pet.html")) {
            Ok(html) => {
                let h1 = Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap();
                let h2 = Header::from_bytes("Cache-Control", "no-store").unwrap();
                let _ = req.respond(Response::from_string(html).with_header(h1).with_header(h2));
            }
            Err(_) => {
                let _ = req.respond(Response::from_string("pet.html not found").with_status_code(404));
            }
        }
        return;
    }

    if method == Method::Get && path == "/pet-sprite.webp" {
        match sprite_bytes() {
            Some(b) => {
                let h = Header::from_bytes("Content-Type", "image/webp").unwrap();
                let _ = req.respond(Response::from_data(b.clone()).with_header(h));
            }
            None => {
                let _ = req.respond(Response::from_string("sprite not found").with_status_code(404));
            }
        }
        return;
    }

    if method == Method::Get && path == "/api/backup" {
        let state = shared.lock().unwrap();
        let payload = json!({
            "version": 1,
            "exportedAt": iso_date(0),
            "tasks": state.get("tasks").cloned().unwrap_or(json!([])),
            "bindings": state.get("bindings").cloned().unwrap_or(json!({}))
        });
        let h1 = Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap();
        let h2 = Header::from_bytes("Content-Disposition", "attachment; filename=\"agent-console-backup.json\"").unwrap();
        let _ = req.respond(Response::from_string(payload.to_string()).with_header(h1).with_header(h2));
        return;
    }

    let (code, resp) = match (&method, path.as_str()) {
        (&Method::Get, p) if p.starts_with("/api/") => handle_api(shared, p, &json!({})),
        (&Method::Post, p) if p.starts_with("/api/") => handle_api(shared, p, &body_json),
        _ => (404, json!({ "ok": false, "reason": "not_found" })),
    };
    json_response(req, code, resp);
}

static SPRITE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();

fn sprite_bytes() -> Option<&'static Vec<u8>> {
    let _ = SPRITE.get_or_init(|| {
        std::fs::read(home_dir()
            .join(".dsh")
            .join("profiles/web/node_modules/@linxin666/dsh-pet/assets/whale-refined/spritesheet.webp"))
        .unwrap_or_default()
    });
    SPRITE.get().filter(|v| !v.is_empty())
}

static APP: std::sync::OnceLock<Mutex<Option<tauri::AppHandle>>> = std::sync::OnceLock::new();

use tauri::Manager;

pub fn set_app_handle(h: tauri::AppHandle) {
    let _ = APP.set(Mutex::new(Some(h)));
}

fn pet_toggle() -> Value {
    let visible;
    if let Some(guard) = APP.get().and_then(|m| m.lock().ok()) {
        if let Some(app) = guard.as_ref() {
            if let Some(pet) = app.get_webview_window("pet") {
                visible = pet.is_visible().unwrap_or(false);
                let _ = if visible { pet.hide() } else { pet.show() };
                return json!({ "ok": true, "visible": !visible });
            }
        }
    }
    json!({ "ok": false, "reason": "no_window" })
}

fn pet_start_drag() -> Value {
    // 完全自研的拖拽：读全局鼠标坐标 + 检测左键，跟着光标 set_position。
    // 不依赖 Tauri 的 start_dragging（WebView2 鼠标捕获下不可靠）。
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

        let app = match APP.get().and_then(|m| m.lock().ok()).and_then(|g| g.clone()) {
            Some(a) => a,
            None => return json!({ "ok": false, "reason": "no_app" }),
        };
        let pet = match app.get_webview_window("pet") {
            Some(p) => p,
            None => return json!({ "ok": false, "reason": "no_window" }),
        };
        let win_pos = pet.outer_position().ok();
        let mut pt = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut pt); }
        let (dx, dy) = match win_pos {
            Some(p) => (pt.x - p.x, pt.y - p.y),
            None => (pt.x - 96, pt.y - 104),
        };
        std::thread::spawn(move || {
            loop {
                unsafe {
                    if (GetAsyncKeyState(0x01) as u16 & 0x8000u16) == 0 {
                        break;
                    }
                    let mut p = POINT { x: 0, y: 0 };
                    if GetCursorPos(&mut p) != 0 {
                        if let Some(w) = app.get_webview_window("pet") {
                            let _ = w.set_position(tauri::PhysicalPosition::new(p.x - dx, p.y - dy));
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
    }
    json!({ "ok": true })
}

fn main_show() -> Value {
    if let Some(guard) = APP.get().and_then(|m| m.lock().ok()) {
        if let Some(app) = guard.as_ref() {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
                return json!({ "ok": true });
            }
        }
    }
    json!({ "ok": false, "reason": "no_window" })
}

static DS_CACHE: std::sync::OnceLock<Mutex<Option<(std::time::Instant, Value)>>> =
    std::sync::OnceLock::new();

fn fetch_dshbalance() -> Value {
    let cache = DS_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(c) = cache.lock() {
        if let Some((at, v)) = c.as_ref() {
            if at.elapsed() < Duration::from_secs(60) {
                return v.clone();
            }
        }
    }
    let key = match cred_key("DEEPSEEK_API_KEY") {
        Some(k) => k,
        None => return json!({ "ok": false, "reason": "no_key" }),
    };
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).build();
    let resp = agent
        .get("https://api.deepseek.com/user/balance")
        .set("Authorization", &format!("Bearer {}", key))
        .call();
    let out = match resp {
        Ok(r) => {
            let mut s = String::new();
            let _ = r.into_reader().read_to_string(&mut s);
            let v: Value = serde_json::from_str(&s).unwrap_or(json!({}));
            match (v.get("is_available"), v.get("balance_infos")) {
                (Some(avail), Some(infos)) if infos.is_array() => json!({
                    "ok": true, "isAvailable": avail, "balances": infos
                }),
                _ => json!({ "ok": false, "reason": "invalid", "detail": s.chars().take(200).collect::<String>() }),
            }
        }
        Err(e) => json!({ "ok": false, "reason": "network", "detail": e.to_string() }),
    };
    if let Ok(mut c) = cache.lock() {
        *c = Some((std::time::Instant::now(), out.clone()));
    }
    out
}

fn log_line(msg: &str) {
    use std::io::Write;
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true).append(true)
        .open(project_dir().join("desktop.log"))
    {
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
}

pub fn start() -> Result<(), String> {
    let server = Server::http("127.0.0.1:8766").map_err(|e| e.to_string())?;
    let server = Arc::new(server);
    let shared: Arc<Mutex<Value>> = Arc::new(Mutex::new(load_state()));
    for _ in 0..4 {
        let s = server.clone();
        let sh = shared.clone();
        std::thread::spawn(move || loop {
            match s.recv() {
                Ok(req) => {
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        handle(req, &sh);
                    }));
                    if r.is_err() {
                        log_line("request handler panicked (recovered)");
                    }
                }
                Err(e) => {
                    // Windows 上 accept 偶发瞬时错误（如 WSAECONNRESET）——不能退出线程，
                    // 否则 Arc<Server> 归零会关闭监听端口，整个服务假死
                    log_line(&format!("recv error: {} (worker survives)", e));
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        });
    }
    log_line("http server started on 127.0.0.1:8766");
    Ok(())
}
