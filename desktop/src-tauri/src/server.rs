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
        // 菜单改用「独立小窗」而不是给宠物窗改尺寸：
        // 透明分层窗 + resizable/set_size 会让 WebView2 子进程崩溃（实测踩过，见交接文档）。
        "/api/menu/open" => (200, menu_open()),
        "/api/menu/close" => (200, menu_close()),
        "/api/quit" => {
            // 优雅退出。存在的意义是**替代 taskkill /F**：
            // 强杀会让 WebView2 的子进程变孤儿、用户数据目录处于不一致状态
            // （2026-09-10 那次白屏事故就是这么被反复强杀累积出来的）。
            // 配合项目根的 restart.cmd 使用：quit → 等端口释放 → 重新拉起 exe。
            let app = APP.get().and_then(|m| m.lock().ok()).and_then(|g| g.clone());
            match app {
                Some(app) => {
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(150)); // 先让响应发出去
                        app.exit(0);
                    });
                    (200, json!({ "ok": true }))
                }
                None => (200, json!({ "ok": false, "reason": "no_app" })),
            }
        }
        // 实时网速：端点只读后台快照，绝不在请求里现采样（见文件顶部铁律 2）
        "/api/netspeed" => (200, netspeed_json(false)),
        "/api/netspeed/detail" => (200, netspeed_json(true)),
        "/api/net/toggle" => {
            // 一键隐藏/显示网速气泡；状态落盘，重启后保持。
            // 不传 enabled 就取反（菜单里那一项直接 POST 空 body）
            let cur = net_enabled();
            let want = body.get("enabled").and_then(|v| v.as_bool());
            (200, net_set_enabled(want.unwrap_or(!cur)))
        }
        "/api/pet/geom" => (200, pet_geom()),
        "/api/pet/move" => {
            // 直接把宠物窗移到指定屏幕坐标（物理像素）。用于 JS 驱动拖拽与排障。
            let x = body.get("x").and_then(|v| v.as_i64()).unwrap_or(i64::MIN);
            let y = body.get("y").and_then(|v| v.as_i64()).unwrap_or(i64::MIN);
            (200, pet_move(x, y))
        }
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
        log_line("page fetch: /index.html");
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

    // 白名单静态文件：文件名取自这张表，不拼接客户端输入，天然免疫路径穿越。
    // pet.json / pet-behavior.json 走这里下发 → 改动作、调节奏、加台词都不需要重新编译。
    const STATIC: &[(&str, &str)] = &[
        ("/pet.html", "text/html; charset=utf-8"),
        ("/menu.html", "text/html; charset=utf-8"),
        ("/net.html", "text/html; charset=utf-8"),
        ("/pet.json", "application/json; charset=utf-8"),
        ("/pet-behavior.json", "application/json; charset=utf-8"),
    ];
    if method == Method::Get {
        if let Some(&(name, ctype)) = STATIC.iter().find(|(p, _)| *p == path.as_str()) {
            log_line(&format!("static: {}", name));
            let file = project_dir().join("public").join(name.trim_start_matches('/'));
            match std::fs::read_to_string(&file) {
                Ok(body) => {
                    let h1 = Header::from_bytes("Content-Type", ctype).unwrap();
                    let h2 = Header::from_bytes("Cache-Control", "no-store").unwrap();
                    let _ = req.respond(Response::from_string(body).with_header(h1).with_header(h2));
                }
                Err(_) => {
                    let _ = req.respond(Response::from_string(format!("{} not found", name)).with_status_code(404));
                }
            }
            return;
        }
    }

    if method == Method::Get && path == "/pet-sprite.webp" {
        match sprite_bytes() {
            Some(b) => {
                let h = Header::from_bytes("Content-Type", "image/webp").unwrap();
                let _ = req.respond(Response::from_data(b.to_vec()).with_header(h));
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

// 宠物雪碧图（1536x1872，8列x9行，cell 192x208）。
// 【2026-09-10 加固】原先只在运行时读 dsh-pet 插件目录，一旦卸载/升级 dsh-pet 或改动
// web profile，宠物立即 404。现改为编译期 include_bytes! 内嵌，外部依赖彻底消除。
// 若仍想「不重编译就换素材」：把新图放到下面的外部覆盖路径，重启应用即生效；
// 该文件不存在或读取失败时，自动回落到内嵌素材，永不报错。
static SPRITE_EMBEDDED: &[u8] = include_bytes!("../assets/spritesheet.webp");
static SPRITE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();

fn sprite_bytes() -> Option<&'static [u8]> {
    let v = SPRITE.get_or_init(|| {
        // 1) 项目内 public/spritesheet.webp —— 换新图只需替换这个文件并重启应用，**无需重新编译**
        let local = project_dir().join("public").join("spritesheet.webp");
        if let Ok(b) = std::fs::read(&local) {
            if !b.is_empty() {
                return b;
            }
        }
        // 2) dsh-pet 插件目录（便于直接取用官方素材）
        let external = home_dir().join(".dsh").join("profiles/web/node_modules")
            .join("@linxin666/dsh-pet/assets/whale-refined/spritesheet.webp");
        match std::fs::read(&external) {
            Ok(b) if !b.is_empty() => b,
            // 3) 编译期内嵌兜底：永不缺失
            _ => SPRITE_EMBEDDED.to_vec(),
        }
    });
    if v.is_empty() {
        None
    } else {
        Some(v.as_slice())
    }
}

/* ==================== 实时网速（GetIfTable2 差分采样） ====================
   四条铁律（详见 skill `windows-realtime-netspeed`）：
   1) 必须排除环回接口(Type=24) —— 否则应用自己每秒轮询走环回，空闲时会有恒定底噪
   2) 绝不能"有请求才采样" —— 多消费者会互相截断时间窗，dt≈0 让速率失真；
      所以这里由后台线程固定 1Hz 采样，HTTP 端点只读快照
   3) 不按"默认路由"挑网卡 —— 实测本机流量走的网卡与路由表指向的不一致
   4) 分项明细当一等公民 —— VPN/TUN 双计数只能靠明细看穿
   口径：汇总所有「OperStatus=Up 且 非环回」网卡的增量（用户选定，不漏） */

const NET_SAMPLE_MS: u64 = 1000;
// 气泡窗尺寸（逻辑像素）：166x56 = 气泡卡 158x52 + 左侧 8px 尾巴让位 + 上下各 2px 余量。
// 这样"透明死区"几乎等于气泡本身，不会像"把宠物窗加宽"那样多出一大片挡点击的区域。
const NET_WIN_W_LP: i32 = 166;
const NET_WIN_H_LP: i32 = 56;
const NET_GAP_LP: i32 = 2;       // 窗口左缘与宠物窗右缘的间隙；实际视觉间距 ≈ 2+8(尾巴)=10
const NET_HEAD_TOP_LP: i32 = 39; // 气泡顶边相对「宠物画布顶边」的偏移（与头部齐平）
const NET_CANVAS_TOP_LP: i32 = 72; // 宠物窗内画布顶部留白（窗口 280 - 画布 208）

#[derive(Clone, Copy)]
struct NetSnap {
    down: f64,   // 字节/秒
    up: f64,
    at: std::time::Instant,
}

static NET_SNAP: std::sync::OnceLock<Mutex<Option<NetSnap>>> = std::sync::OnceLock::new();
static NET_DETAIL: std::sync::OnceLock<Mutex<Vec<(String, f64, f64)>>> = std::sync::OnceLock::new();
static NET_ON: std::sync::OnceLock<Mutex<bool>> = std::sync::OnceLock::new();
static NET_SIDE: std::sync::OnceLock<Mutex<bool>> = std::sync::OnceLock::new(); // true=贴在宠物右侧

fn net_cfg_path() -> PathBuf {
    project_dir().join("net-bubble.json")
}
fn net_state() -> &'static Mutex<bool> {
    NET_ON.get_or_init(|| {
        let v = read_json(&net_cfg_path(), json!({ "enabled": true }));
        Mutex::new(v.get("enabled").and_then(|b| b.as_bool()).unwrap_or(true))
    })
}
fn net_enabled() -> bool {
    *net_state().lock().unwrap()
}
fn net_set_enabled(on: bool) -> Value {
    *net_state().lock().unwrap() = on;
    write_json(&net_cfg_path(), &json!({ "enabled": on }));
    json!({ "ok": true, "enabled": on })
}

/// 读一次各网卡累计字节。返回 (接口名, 累计收, 累计发)，已过滤环回与非 Up。
#[cfg(windows)]
unsafe fn if_totals() -> Option<Vec<(String, u64, u64)>> {
    use windows_sys::Win32::Foundation::NO_ERROR;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetIfTable2, IF_TYPE_SOFTWARE_LOOPBACK, MIB_IF_TABLE2,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;

    let mut p: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    if GetIfTable2(&mut p) != NO_ERROR || p.is_null() {
        return None;
    }
    let t = &*p;
    // Table 声明为 [MIB_IF_ROW2; 1]，真实长度由 NumEntries 决定
    let rows = std::slice::from_raw_parts(t.Table.as_ptr(), t.NumEntries as usize);
    let mut out: Vec<(String, u64, u64)> = Vec::with_capacity(rows.len());
    for r in rows {
        if r.OperStatus != IfOperStatusUp {
            continue;
        }
        if r.Type == IF_TYPE_SOFTWARE_LOOPBACK {
            continue; // ← 铁律 1
        }
        // ← 铁律 5（实测踩出来的）：排除 NDIS 过滤层接口。
        // 每张真实网卡在 GetIfTable2 里会被它的过滤层再"复制"若干份，别名形如
        // "<真实别名>-WFP Native MAC Layer LightWeight Filter-0000" / "-QoS Packet Scheduler-0000" /
        // "-Native WiFi Filter Driver-0000"。这些层上报的累计字节与真实网卡**完全相同**，
        // 于是同一份流量被算 5 遍（实测：WLAN 真速 65 K/s 被报成 328 K/s，正好 5×）。
        // MIB_IF_ROW2_0 是位域：HardwareInterface:1, FilterInterface:1, ...（MSVC 从低位起排）
        // → FilterInterface = bit1 = 0x02
        if r.InterfaceAndOperStatusFlags._bitfield & 0x02 != 0 {
            continue;
        }
        let cut = |a: &[u16]| -> String {
            let n = a.iter().position(|&c| c == 0).unwrap_or(a.len());
            String::from_utf16_lossy(&a[..n])
        };
        let name = cut(&r.Alias);
        let name = if name.is_empty() { cut(&r.Description) } else { name };
        out.push((name, r.InOctets, r.OutOctets));
    }
    FreeMibTable(p as *const core::ffi::c_void);
    Some(out)
}

#[cfg(not(windows))]
unsafe fn if_totals() -> Option<Vec<(String, u64, u64)>> {
    None
}

/// 一个采样节拍：算增量 → 写快照 + 明细
fn netspeed_tick(prev: &mut BTreeMap<String, (u64, u64)>, prev_at: &mut Option<std::time::Instant>) {
    let rows = match unsafe { if_totals() } {
        Some(r) => r,
        None => return,
    };
    let now = std::time::Instant::now();
    // 首拍把"参与统计的网卡"写进日志 —— 日后若读数偏高，看一眼日志就知道是谁在计数
    if prev_at.is_none() {
        let names: Vec<String> = rows.iter().map(|(n, _, _)| n.clone()).collect();
        log_line(&format!("netspeed counting {} iface(s): {}", names.len(), names.join(" | ")));
    }
    let mut per: Vec<(String, f64, f64)> = Vec::new();
    if let Some(t0) = *prev_at {
        let dt = now.duration_since(t0).as_secs_f64().max(0.001);
        for (name, rx, tx) in &rows {
            // 只对"上一拍就见过"的网卡算增量；新出现的网卡本次跳过，
            // 否则会把它的历史累计值当成"这一秒的流量"（会瞬间爆表）
            if let Some((prx, ptx)) = prev.get(name) {
                let d = rx.saturating_sub(*prx) as f64 / dt;
                let u = tx.saturating_sub(*ptx) as f64 / dt;
                if d >= 1.0 || u >= 1.0 {
                    per.push((name.clone(), d, u));
                }
            }
        }
    }
    let mut cur = BTreeMap::new();
    for (name, rx, tx) in rows {
        cur.insert(name, (rx, tx));
    }
    *prev = cur;
    *prev_at = Some(now);

    per.sort_by(|a, b| (b.1 + b.2).partial_cmp(&(a.1 + a.2)).unwrap_or(std::cmp::Ordering::Equal));
    let down: f64 = per.iter().map(|x| x.1).sum();
    let up: f64 = per.iter().map(|x| x.2).sum();
    *NET_SNAP.get_or_init(|| Mutex::new(None)).lock().unwrap() = Some(NetSnap { down, up, at: now });
    *NET_DETAIL.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap() = per;
}

/// 字节/秒 → 显示文本（用户选定 MB/s 口径，三位有效数字防宽度跳动）
fn fmt_speed(bps: f64) -> String {
    if !bps.is_finite() || bps < 0.5 {
        return "0 K/s".to_string();
    }
    if bps < 1024.0 {
        return format!("{} B/s", bps.round() as u64);
    }
    let kb = bps / 1024.0;
    if kb < 1024.0 {
        return format!("{} K/s", kb.round() as u64);
    }
    let mb = kb / 1024.0;
    if mb < 100.0 {
        format!("{:.2} M/s", mb)
    } else {
        format!("{:.1} M/s", mb)
    }
}

fn netspeed_json(with_detail: bool) -> Value {
    let snap = *NET_SNAP.get_or_init(|| Mutex::new(None)).lock().unwrap();
    let (down, up, warm, stale, age) = match snap {
        Some(s) => {
            let age = s.at.elapsed().as_secs_f64();
            (s.down, s.up, false, age > 5.0, age)
        }
        // 启动首拍还没有"上一拍"，标记 warm（前端显示占位符而不是 0）
        None => (0.0, 0.0, true, false, 0.0),
    };
    let mut v = json!({
        "ok": true,
        "down": down,
        "up": up,
        "downText": if warm { "—".to_string() } else { fmt_speed(down) },
        "upText": if warm || stale { "—".to_string() } else { fmt_speed(up) },
        "warm": warm,
        "stale": stale,
        "age": age,
        "scope": "all",
        "enabled": net_enabled(),
        "side": if *NET_SIDE.get_or_init(|| Mutex::new(true)).lock().unwrap() { "right" } else { "left" },
        "sampleMs": NET_SAMPLE_MS,
    });
    if with_detail {
        let list = NET_DETAIL.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap().clone();
        let ifaces: Vec<Value> = list.iter().map(|(n, d, u)| json!({
            "name": n, "down": d, "up": u,
            "downText": fmt_speed(*d), "upText": fmt_speed(*u)
        })).collect();
        v.as_object_mut().unwrap().insert("ifaces".to_string(), json!(ifaces));
    }
    v
}

/// 网速气泡跟随宠物：贴右侧、与头部齐平；右侧放不下就翻到左侧
fn net_place(pet: &tauri::WebviewWindow, net: &tauri::WebviewWindow) -> Option<(i32, i32)> {
    let p = pet.outer_position().ok()?;
    let ps = pet.outer_size().ok()?;
    let ns = net.outer_size().ok()?;
    let mon = pet.current_monitor().ok().flatten()?;
    let sf = mon.scale_factor();
    let gap = (NET_GAP_LP as f64 * sf).round() as i32;
    let head = ((NET_CANVAS_TOP_LP + NET_HEAD_TOP_LP) as f64 * sf).round() as i32;
    let y = p.y + head;
    let right_x = p.x + ps.width as i32 + gap;
    let mon_left = mon.position().x;
    let mon_right = mon_left + mon.size().width as i32;
    let (x, side_right) = if right_x + ns.width as i32 <= mon_right - 4 {
        (right_x, true) // 正常：贴在宠物右侧
    } else {
        ((p.x - gap - ns.width as i32).max(mon_left + 4), false) // 越界保护：翻到左侧
    };
    *NET_SIDE.get_or_init(|| Mutex::new(true)).lock().unwrap() = side_right;
    let _ = net.set_position(tauri::PhysicalPosition::new(x, y));
    Some((x, y))
}

/// 按需创建网速气泡窗。
/// ⚠️ 为什么不在 tauri.conf.json 里声明、而要运行时建：
/// 实测（2026-09-18）把第 4 个透明窗写进配置后，Tauri 启动时**卡死在创建这个窗上** ——
/// 日志显示 main/pet/petmenu 都正常取了页面，唯独没有 `/net.html` 请求，
/// 于是 setup() 永不执行（tray 没装、宠物没落位、`/api/pet/*` 全返回 no_app）。
/// 改成从跟随线程里建：setup 必定跑完，最坏情况只是气泡不出现，主体功能不受影响。
fn ensure_net_window(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(w) = app.get_webview_window("net") {
        return Some(w);
    }
    let url: tauri::Url = match "http://127.0.0.1:8766/net.html".parse() {
        Ok(u) => u,
        Err(_) => return None,
    };
    let built = tauri::WebviewWindowBuilder::new(app, "net", tauri::WebviewUrl::External(url))
        .title("网速")
        .inner_size(NET_WIN_W_LP as f64, NET_WIN_H_LP as f64)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .resizable(false)
        .focused(false)
        .visible(false)
        .additional_browser_args("--disable-gpu --disable-features=RendererCodeIntegrity")
        .build();
    match built {
        Ok(w) => {
            log_line("net window created (lazy)");
            Some(w)
        }
        Err(e) => {
            log_line(&format!("net window create failed: {}", e));
            None
        }
    }
}

/// 宠物窗 + 网速气泡「整组」贴屏幕右下角（右 22 / 底 120）。
/// 放在 server.rs 而非 main.rs，是为了复用 NET_GAP_LP，不在两处重复硬编码间隙。
/// ⚠️ 必须把气泡宽度算进右边距 —— 否则宠物仍停在原处、气泡会被推出屏幕右侧。
pub fn place_pet_home() {
    let app = match APP.get().and_then(|m| m.lock().ok()).and_then(|g| g.clone()) {
        Some(a) => a,
        None => return,
    };
    let pet = match app.get_webview_window("pet") {
        Some(p) => p,
        None => return,
    };
    let (ps, mon) = match (pet.outer_size().ok(), pet.current_monitor().ok().flatten()) {
        (Some(s), Some(m)) => (s, m),
        _ => return,
    };
    let sf = mon.scale_factor();
    // 用常量而不是查窗口：气泡窗是**运行时才建**的，setup 阶段它还不存在，
    // 若此时按 0 计算，宠物就不会给气泡让位，气泡一出现就会被推出屏幕。
    let net_w = if net_enabled() { NET_WIN_W_LP } else { 0 };
    let gap = if net_w > 0 { (NET_GAP_LP as f64 * sf).round() as i32 } else { 0 };
    let margin_r = (22.0 * sf).round() as i32;
    let margin_b = (120.0 * sf).round() as i32;
    let x = mon.position().x + mon.size().width as i32 - (ps.width as i32 + gap + net_w) - margin_r;
    let y = mon.position().y + mon.size().height as i32 - ps.height as i32 - margin_b;
    let _ = pet.set_position(tauri::PhysicalPosition::new(x, y));
}

/// 常驻跟随线程：宠物一挪窝（拖拽 / /api/pet/move / 隐藏）气泡就跟上。
/// 独立于拖拽实现，所以任何移动路径都覆盖得到。
pub fn start_net_follow() {
    std::thread::spawn(move || {
        let mut applied_enabled: Option<bool> = None;
        let mut last_key: Option<(i32, i32, bool)> = None;
        let mut retry_at = std::time::Instant::now();
        loop {
            let enabled = net_enabled();
            if Some(enabled) != applied_enabled {
                applied_enabled = Some(enabled);
                last_key = None;
                retry_at = std::time::Instant::now();
            }
            // 先把 AppHandle 克隆出来再放锁：建窗可能耗时，绝不占着 APP 的锁
            let handle = APP.get().and_then(|m| m.lock().ok()).and_then(|g| g.clone());
            if let Some(app) = handle {
                if !enabled {
                    // 只在状态切换后隐藏一次（(0,0,false) 作为"已隐藏"哨兵）
                    if last_key.is_none() {
                        if let Some(net) = app.get_webview_window("net") {
                            let _ = net.hide();
                        }
                        last_key = Some((0, 0, false));
                    }
                } else if let Some(pet) = app.get_webview_window("pet") {
                    let mut net = app.get_webview_window("net");
                    if net.is_none() && std::time::Instant::now() >= retry_at {
                        net = ensure_net_window(&app);
                        // 建失败就 3 秒后再试，别每 80ms 刷一遍日志/重试
                        retry_at = std::time::Instant::now() + Duration::from_secs(3);
                    }
                    if let Some(net) = net {
                        let p = pet.outer_position().ok();
                        let vis = pet.is_visible().unwrap_or(false);
                        let key = (
                            p.as_ref().map(|q| q.x).unwrap_or(0),
                            p.as_ref().map(|q| q.y).unwrap_or(0),
                            vis,
                        );
                        if Some(key) != last_key {
                            last_key = Some(key);
                            if vis {
                                let _ = net_place(&pet, &net);
                                let _ = net.show();
                            } else {
                                let _ = net.hide();
                            }
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(80));
        }
    });
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

// 打开鲸鱼娘菜单窗（独立小窗，不改宠物窗尺寸）。
// 位置：贴着宠物窗的左侧、与宠物底边对齐，视觉上像从宠物身上弹出来的。
fn menu_open() -> Value {
    let app = match APP.get().and_then(|m| m.lock().ok()).and_then(|g| g.clone()) {
        Some(a) => a,
        None => return json!({ "ok": false, "reason": "no_app" }),
    };
    let pet = match app.get_webview_window("pet") {
        Some(p) => p,
        None => return json!({ "ok": false, "reason": "no_pet" }),
    };
    let menu = match app.get_webview_window("petmenu") {
        Some(m) => m,
        None => return json!({ "ok": false, "reason": "no_menu_window" }),
    };
    let mut at = None;
    if let (Ok(p), Ok(s), Ok(ms)) = (pet.outer_position(), pet.outer_size(), menu.outer_size()) {
        // 左移一个菜单窗宽，再回退 12px 贴住宠物窗（那 12px 是宠物窗左侧的透明留白）
        let x = p.x - ms.width as i32 + 12;
        // 底边与宠物窗对齐
        let y = p.y + s.height as i32 - ms.height as i32;
        let _ = menu.set_position(tauri::PhysicalPosition::new(x, y));
        at = Some(json!({ "x": x, "y": y }));
    }
    let _ = menu.show();
    let _ = menu.set_focus();
    json!({ "ok": true, "at": at })
}

fn menu_close() -> Value {
    if let Some(guard) = APP.get().and_then(|m| m.lock().ok()) {
        if let Some(app) = guard.as_ref() {
            if let Some(menu) = app.get_webview_window("petmenu") {
                let _ = menu.hide();
                return json!({ "ok": true });
            }
        }
    }
    json!({ "ok": false, "reason": "no_window" })
}

// 宠物窗几何诊断：位置 / 尺寸 / 可见性
fn pet_geom() -> Value {    let app = match APP.get().and_then(|m| m.lock().ok()).and_then(|g| g.clone()) {
        Some(a) => a,
        None => return json!({ "ok": false, "reason": "no_app" }),
    };
    let pet = match app.get_webview_window("pet") {
        Some(p) => p,
        None => return json!({ "ok": false, "reason": "no_window" }),
    };
    let pos = pet.outer_position().ok();
    let size = pet.outer_size().ok();
    json!({
        "ok": true,
        "position": pos.map(|p| json!({ "x": p.x, "y": p.y })),
        "size": size.map(|s| json!({ "w": s.width, "h": s.height })),
        "visible": pet.is_visible().unwrap_or(false),
        "monitor": pet.current_monitor().ok().flatten().map(|m| json!({
            "w": m.size().width, "h": m.size().height, "scale": m.scale_factor()
        }))
    })
}

// 把宠物窗移到指定物理坐标
fn pet_move(x: i64, y: i64) -> Value {
    if x == i64::MIN || y == i64::MIN {
        return json!({ "ok": false, "reason": "bad_pos" });
    }
    let app = match APP.get().and_then(|m| m.lock().ok()).and_then(|g| g.clone()) {
        Some(a) => a,
        None => return json!({ "ok": false, "reason": "no_app" }),
    };
    let pet = match app.get_webview_window("pet") {
        Some(p) => p,
        None => return json!({ "ok": false, "reason": "no_window" }),
    };
    let _ = pet.set_position(tauri::PhysicalPosition::new(x as i32, y as i32));
    json!({ "ok": true, "at": { "x": x, "y": y } })
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
    // 网速采样线程：固定 1Hz，与任何 HTTP 请求无关（端点只读它的产物）
    std::thread::spawn(move || {
        let mut prev: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        let mut prev_at: Option<std::time::Instant> = None;
        loop {
            netspeed_tick(&mut prev, &mut prev_at);
            std::thread::sleep(Duration::from_millis(NET_SAMPLE_MS));
        }
    });
    log_line("http server started on 127.0.0.1:8766");
    Ok(())
}
