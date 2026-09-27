@echo off
chcp 65001 >nul
setlocal
cd /d "%~dp0"

echo ==================================================
echo  上传到 GitHub（一次性配置）
echo ==================================================
echo.
echo 本机网络现状：github.com:443 不通，但 SSH(22) 通
echo   → 推荐用 SSH 方式（脚本会提示先生成密钥）
echo.
echo [当前远程配置]
git remote -v
echo.

echo [1/3] 粘贴你的仓库地址（二选一）：
echo    SSH : git@github.com:你的用户名/agent-console.git
echo    HTTPS: https://github.com/你的用户名/agent-console.git
set /p URL=地址（直接回车 = 不改远程，仅执行推送）: 

if not "%URL%"=="" (
  git remote remove origin 2>nul
  git remote add origin "%URL%"
  echo 已设置远程 origin
)

echo.
echo [2/3] 推送 main 分支...
git push -u origin main

echo.
echo [3/3] 结束。常见问题：
echo   * SSH 报 Permission denied (publickey)：先运行 setup-github-ssh.cmd 生成密钥并把公钥加到 GitHub
echo   * HTTPS 卡住/超时：说明 github.com 不可达，请先开代理，或改用 SSH
echo   * 若仓库尚未创建：先去 github.com 新建同名空仓库（不要初始化 README）
echo.
pause
