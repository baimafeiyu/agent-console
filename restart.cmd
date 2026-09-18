@echo off
rem ============================================================
rem  Restart agent-console gracefully.
rem  Why: force-killing (taskkill /F) leaves WebView2 child
rem  processes orphaned and its user-data dir inconsistent,
rem  which has caused white-screen failures before.
rem  This script asks the app to exit cleanly, waits for the
rem  port to be released, then relaunches the exe.
rem ============================================================
setlocal
set "ROOT=%~dp0"
set "EXE=%ROOT%desktop\src-tauri\target\release\agent-console.exe"

if not exist "%EXE%" (
  echo [x] exe not found: %EXE%
  pause
  exit /b 1
)

echo [1/3] asking app to quit gracefully ...
curl -s -X POST http://127.0.0.1:8766/api/quit >nul 2>&1

echo [2/3] waiting for port 8766 to be released ...
for /l %%i in (1,1,20) do (
  netstat -ano | findstr "8766" | findstr "LISTENING" >nul 2>&1
  if errorlevel 1 goto :up
  ping -n 1 -w 400 127.0.0.1 >nul
)
echo [!] port still busy - the app may not have been running, continuing anyway

:up
echo [3/3] launching ...
start "" "%EXE%"
echo [ok] agent-console restarted
