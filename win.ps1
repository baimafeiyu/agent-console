# agent-console 窗口控制层（供 server.js 调用）
#   powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File win.ps1 -Action <list|procs|alive|focus> [-ProcId N] [-Title s] [-Name s]
#
# 设计原则：
#   1) 列举/探测只用 Get-Process —— 零风险、零编译
#   2) 置顶采用三级降级：COM AppActivate → P/Invoke SetForegroundWindow → 明确报错
#      前两级在 AI 沙箱内被安全策略拦截，但由用户自己启动的 node 进程调用时可用
param(
    [ValidateSet("list", "focus", "alive", "procs", "clip")]
    [string]$Action = "list",
    [int]$ProcId = 0,
    [string]$Title = "",
    [string]$Name = "",
    [string]$Text = ""
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Emit($obj) { ConvertTo-Json -InputObject $obj -Compress -Depth 5 }

# ---------- 置顶：P/Invoke（最强，能还原最小化窗口） ----------
function Focus-ByPInvoke {
    param([int]$TargetPid, [string]$TargetTitle)
    $src = @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public class WA {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    public delegate bool EWP(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EWP f, IntPtr l);
    public static long Find(int pid, string title) {
        long found = 0;
        EnumWindows(delegate (IntPtr h, IntPtr l) {
            if (found != 0) return true;
            if (!IsWindowVisible(h)) return true;
            var sb = new StringBuilder(512);
            GetWindowText(h, sb, 512);
            var t = sb.ToString();
            uint p = 0; GetWindowThreadProcessId(h, out p);
            bool hit = false;
            if (pid > 0 && (int)p == pid) hit = true;
            if (!hit && !string.IsNullOrEmpty(title) && t.IndexOf(title, StringComparison.OrdinalIgnoreCase) >= 0) hit = true;
            if (hit) found = (long)h;
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static bool Focus(long hwnd) {
        IntPtr h = new IntPtr(hwnd);
        if (!IsWindow(h)) return false;
        if (IsIconic(h)) ShowWindow(h, 9);
        BringWindowToTop(h);
        return SetForegroundWindow(h);
    }
}
'@
    if (-not ('WA' -as [type])) { Add-Type -TypeDefinition $src -Language CSharp | Out-Null }
    $hwnd = [WA]::Find($TargetPid, $TargetTitle)
    if ($hwnd -eq 0) { return @{ ok = $false; reason = 'no_window' } }
    return @{ ok = [WA]::Focus($hwnd); hwnd = $hwnd; via = 'pinvoke' }
}

# ---------- 置顶：COM AppActivate（轻量，能还原最小化） ----------
function Focus-ByCom {
    param([int]$TargetPid, [string]$TargetTitle)
    $ws = New-Object -ComObject WScript.Shell
    if ($TargetPid -gt 0) { return $ws.AppActivate($TargetPid) }
    return $ws.AppActivate($TargetTitle)
}

switch ($Action) {

    'list' {
        $arr = @()
        foreach ($p in (Get-Process | Where-Object { $_.MainWindowTitle -ne '' })) {
            $arr += [pscustomobject]@{ pid = $p.Id; name = $p.ProcessName; title = $p.MainWindowTitle }
        }
        Emit @($arr)
    }

    'procs' {
        $arr = @()
        if ($Name -ne '') {
            foreach ($p in (Get-Process -Name $Name -ErrorAction SilentlyContinue)) {
                $arr += [pscustomobject]@{ pid = $p.Id; name = $p.ProcessName; title = $p.MainWindowTitle }
            }
        }
        Emit @($arr)
    }

    'alive' {
        $p = Get-Process -Id $ProcId -ErrorAction SilentlyContinue
        if ($null -eq $p) { Emit @{ alive = $false } }
        else { Emit @{ alive = $true; name = $p.ProcessName; title = $p.MainWindowTitle } }
    }

    'clip' {
        try { Set-Clipboard -Value $Text -ErrorAction Stop; Emit @{ ok = $true } }
        catch { Emit @{ ok = $false; reason = 'clip_failed' } }
    }

    'focus' {
        if ($ProcId -eq 0 -and $Title -eq '') { Emit @{ ok = $false; reason = 'no_target' }; break }

        # 先确认目标进程还活着（顺带拿到可用于 COM 的 pid）
        $targetPid = $ProcId
        if ($targetPid -eq 0 -and $Title -ne '') {
            $hit = Get-Process | Where-Object { $_.MainWindowTitle -ne '' -and $_.MainWindowTitle -like "*$Title*" } | Select-Object -First 1
            if ($null -ne $hit) { $targetPid = $hit.Id } else { Emit @{ ok = $false; reason = 'process_not_found' }; break }
        }

        try {
            $ok = Focus-ByCom -TargetPid $targetPid
            if ($ok) { Emit @{ ok = $true; pid = $targetPid; via = 'com' }; break }
        } catch { }

        try {
            $r = Focus-ByPInvoke -TargetPid $targetPid -TargetTitle $Title
            Emit $r; break
        } catch { }

        Emit @{ ok = $false; pid = $targetPid; reason = 'all_methods_blocked' }
    }
}
