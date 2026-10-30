param(
    [ValidateSet("all", "windows", "linux")]
    [string]$Target = "all"
)

$ErrorActionPreference = "Stop"
$ProjectRoot = $PSScriptRoot

Write-Host "====================================================" -ForegroundColor Cyan
Write-Host " SVCMusicPlayer-rs 自动化双端快速构建流水线" -ForegroundColor Cyan
Write-Host " 目标: $Target" -ForegroundColor Cyan
Write-Host "====================================================" -ForegroundColor Cyan

# 确保交付目录存在
$DistWin = Join-Path $ProjectRoot "dist\windows-x86_64"
$DistLinux = Join-Path $ProjectRoot "dist\linux-amd64"

if ($Target -eq "all" -or $Target -eq "windows") {
    Write-Host "`n>>> [1/2] 正在编译 Windows x86_64 Release 版本..." -ForegroundColor Green
    $startTime = Get-Date

    Push-Location $ProjectRoot
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) {
            throw "Windows Release 编译失败！"
        }
    } finally {
        Pop-Location
    }

    New-Item -ItemType Directory -Force -Path $DistWin | Out-Null
    Copy-Item -Force (Join-Path $ProjectRoot "target\release\svc_music_player.exe") (Join-Path $DistWin "svc_music_player.exe")
    Copy-Item -Force (Join-Path $ProjectRoot "config.example.toml") (Join-Path $DistWin "config.example.toml")

    $elapsed = (Get-Date) - $startTime
    Write-Host ">>> Windows 版本构建完成！耗时: $($elapsed.TotalSeconds.ToString('F1'))s" -ForegroundColor Green
    Write-Host "    产物位置: $DistWin\svc_music_player.exe" -ForegroundColor Gray
}

if ($Target -eq "all" -or $Target -eq "linux") {
    Write-Host "`n>>> [2/2] 正在通过 WSL 编译 Linux amd64 Release 版本 (利用 WSL 原生 ext4 缓存加速)..." -ForegroundColor Green
    $startTime = Get-Date

    # 检查 WSL 可用性
    $wslCheck = wsl -e bash -lc "echo ok" 2>$null
    if ($wslCheck -ne "ok") {
        Write-Warning "未检测到可用的 WSL 环境，跳过 Linux 版本构建。"
    } else {
        # 转换当前路径为 WSL 路径
        $wslPath = (wsl -e wslpath -u ($ProjectRoot.Replace('\', '/'))).Trim()

        New-Item -ItemType Directory -Force -Path $DistLinux | Out-Null

        # 执行 WSL 构建：利用原生 ext4 高速缓存 $HOME/.cache/svc_music_player_target，绕开 9P 文件系统跨端锁与 I/O 瓶颈
        $wslCmd = "export CARGO_TARGET_DIR=`"`$HOME/.cache/svc_music_player_target`" && cd `"$wslPath`" && cargo build --release"
        wsl -e bash -lc $wslCmd

        if ($LASTEXITCODE -ne 0) {
            throw "WSL Linux Release 编译失败！"
        }

        # 拷贝产物至 Windows 下的 dist 目录
        $wslDistPath = (wsl -e wslpath -u ($DistLinux.Replace('\', '/'))).Trim()
        $copyCmd = "cp `"`$HOME/.cache/svc_music_player_target/release/svc_music_player`" `"$wslDistPath/svc_music_player`""
        wsl -e bash -lc $copyCmd

        Copy-Item -Force (Join-Path $ProjectRoot "config.example.toml") (Join-Path $DistLinux "config.example.toml")

        $elapsed = (Get-Date) - $startTime
        Write-Host ">>> Linux 版本构建完成！耗时: $($elapsed.TotalSeconds.ToString('F1'))s" -ForegroundColor Green
        Write-Host "    产物位置: $DistLinux\svc_music_player" -ForegroundColor Gray
    }
}

Write-Host "`n====================================================" -ForegroundColor Cyan
Write-Host " 全部构建任务已圆满完成！" -ForegroundColor Cyan
Write-Host "====================================================" -ForegroundColor Cyan
