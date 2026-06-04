# 强制终止 Tauri 开发进程
Write-Host "正在终止开发进程..." -ForegroundColor Yellow

# 终止所有相关进程（包括整个进程树）
$names = @("ai-balance-widget-new-api", "cargo", "rustc", "CrashSender")
foreach ($name in $names) {
    Get-Process -Name $name -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
}

Write-Host "完成" -ForegroundColor Green
