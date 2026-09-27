@echo off
chcp 65001 >nul
setlocal
cd /d "%~dp0"

echo ==================================================
echo  生成 GitHub SSH 密钥（本机 22 端口可通 GitHub）
echo ==================================================
echo.

if exist "%USERPROFILE%\.ssh\id_ed25519.pub" (
  echo [已存在密钥] 公钥内容如下：
  echo.
  type "%USERPROFILE%\.ssh\id_ed25519.pub"
  echo.
  goto :verify
)

echo [1/2] 生成新密钥（邮箱可用任意值，仅作备注）
set /p EMAIL=邮箱（直接回车用默认 local@local）: 
if "%EMAIL%"=="" set EMAIL=local@local

ssh-keygen -t ed25519 -C "%EMAIL%" -f "%USERPROFILE%\.ssh\id_ed25519" -N ""

echo.
echo [2/2] 公钥（复制下面整行）：
echo --------------------------------------------------
type "%USERPROFILE%\.ssh\id_ed25519.pub"
echo --------------------------------------------------
echo.
echo 把它粘贴到 GitHub：Settings → SSH and GPG keys → New SSH key
echo （github.com 打不开时，可在能联网的设备/手机浏览器上操作）
echo.

:verify
echo 添加完成后按任意键验证连接...
pause >nul
ssh -o StrictHostKeyChecking=accept-new -T git@github.com
echo.
echo 看到 "Hi 你的用户名!" 即成功。然后运行 push-to-github.cmd 推送。
pause
