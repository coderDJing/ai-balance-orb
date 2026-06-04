# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概述

AI Balance Orb 是一个基于 **Tauri v2** 的 Windows 桌面悬浮球小部件，用于轮询 New API 兼容的余额接口并实时显示账户余额。关闭窗口时最小化到系统托盘而非退出。

## 常用命令

```bash
pnpm install              # 安装依赖
pnpm tauri:dev            # 启动完整开发模式（前端 + Rust 后端）
pnpm build                # TypeScript 检查 + Vite 生产构建（仅前端）
pnpm check:desktop        # Rust 类型检查（cargo check）
pnpm tauri:build          # 完整桌面打包
```

没有测试套件。

## 架构

### 技术栈

- **前端：** React 19 + TypeScript 5.8 + Vite 7 + lucide-react
- **后端：** Rust (Tauri v2 + reqwest + serde + tauri-plugin-updater)
- **包管理器：** pnpm（非 npm/yarn）
- **Tauri API：** `@tauri-apps/api` v2（注意不是 v1）

### 代码结构

前端和后端逻辑集中在两个核心文件：

- `src/App.tsx` — 整个前端，包含 `BalanceWindow`（主悬浮窗）和 `SettingsWindow`（设置表单）两个组件。无路由、无状态管理库。
- `src-tauri/src/lib.rs` — 后端逻辑，包含配置读写、余额查询、窗口定位、系统托盘和 signed updater。

### Tauri 配置

`src-tauri/tauri.conf.json` 定义了两个窗口：
1. **main** (220-520×120) — 无边框、透明背景、置顶、不在任务栏显示，宽度由余额位数动态调整
2. **settings** (440×420) — 无边框、透明、初始隐藏

### 关键业务逻辑

- 余额查询：`GET <endpoint_url>/api/user/self` 带 `Authorization: Bearer <token>` 和 `New-Api-User: <userId>` 请求头
- 余额计算：`quota / 500_000`（常量 `QUOTA_SCALE`）
- 配置文件：存储在 Tauri app config 目录的 `config.json`（含 `endpoint_url`、`access_token`、`user_id`）
- 端点校验：必须是 HTTPS 或 localhost
- 前端在非 Tauri 环境（`pnpm dev`）下有 mock 数据，可独立开发 UI

### UI

主悬浮窗使用机械翻页闹钟风格显示余额，余额固定保留三位小数且不使用千分位逗号。界面文字以中文为主。

## CI

`.github/workflows/build.yml` 仅在 Windows 运行：`pnpm build` → `pnpm check:desktop` → `pnpm tauri:build`，产物上传为 GitHub Actions artifact。

## 发布流程

发布流程从 `D:\playground\keyboard-lock-osd` 迁移并按本仓库适配。

### 版本规则

发布前必须保持三个版本号一致：

- `package.json` → `"version": "x.y.z"`
- `src-tauri/Cargo.toml` → `version = "x.y.z"`
- `src-tauri/tauri.conf.json` → `"version": "x.y.z"`

Git tag 必须使用 `v` 前缀，例如版本 `0.1.1` 对应 tag `v0.1.1`。

### 发布脚本

在项目根目录运行：

```powershell
.\scripts\release.ps1 0.1.1
```

脚本会检查当前分支必须是 `master`、工作区必须干净、本地不能落后 `origin/master`，然后同步三个版本文件、运行 `pnpm build`、`pnpm check:desktop` 和 signed debug NSIS updater build，最后提交版本变更、创建 tag、推送 `master` 和 tag。

跳过交互确认：

```powershell
.\scripts\release.ps1 0.1.1 -Force
```

不要在用户没有明确要求时运行该脚本，因为它会执行 `git commit`、`git tag` 和 `git push`。

### GitHub Release

`.github/workflows/release.yml` 在以下场景运行：

- 推送 `v*` tag
- 手动 `workflow_dispatch`

工作流只在 `windows-latest` 构建，使用 `tauri-apps/tauri-action@v0.6.2` 发布 GitHub Release，配置为：

- `releaseDraft: false`
- `prerelease: false`
- `updaterJsonPreferNsis: true`
- `args: --ci`

release workflow 必须配置以下 GitHub repository secrets 才能生成 signed updater 产物：

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

当前本机私钥路径：

```text
C:\Users\coder\.tauri\ai-balance-orb.key
```

该私钥没有密码，`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 可以为空或省略。不要把私钥内容写入仓库。

### Signed Updater

Tauri updater endpoint：

```text
https://github.com/coderDJing/ai-balance-orb/releases/latest/download/latest.json
```

发布产物必须包含 Windows installer、installer signature 和 `latest.json`。release build 启动时会自动检查更新，托盘菜单也提供“检查更新”。
