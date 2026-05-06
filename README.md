# Super AI Operations

运维/部署说明。产品目标、实现约束和 AI 接手规则见 `AGENTS.md`。

## 环境要求

- Node.js 22+
- npm 10+
- Rust stable
- Cargo
- Tauri 2 相关系统依赖
- macOS / Windows 打包签名环境

当前本机已通过 Homebrew 安装 Rust/Cargo，并已初始化 Tauri 2 项目结构。

## 安装依赖

```bash
cd /Users/xusanye/Desktop/ai_manage/ai-auth-studio
npm install
```

## 桌面 APP 开发启动

```bash
cd /Users/xusanye/Desktop/ai_manage/ai-auth-studio
npm run dev
```

`npm run dev` 会启动 Tauri 桌面 APP，并自动拉起 Vite 开发服务器。

热更新说明：

- 修改 `src/**/*.tsx`、`src/**/*.css` 等前端文件后，APP 窗口内会自动热更新。
- 修改 `src-tauri/**` Rust 或 Tauri 配置后，Tauri dev 通常会重新编译并重启窗口。
- 终端不要关，关掉终端就会结束开发 APP。

## 网页预览

```bash
cd /Users/xusanye/Desktop/ai_manage/ai-auth-studio
npm run web:dev -- --host 127.0.0.1
```

这个命令只启动浏览器网页预览，不会打开桌面 APP。一般不推荐日常使用，只用于快速调试纯前端布局。

默认访问地址：

```text
http://127.0.0.1:5173/
```

## 生产构建

```bash
cd /Users/xusanye/Desktop/ai_manage/ai-auth-studio
npm run build
```

构建产物目录：

```text
/Users/xusanye/Desktop/ai_manage/ai-auth-studio/dist
```

这个命令只构建前端静态文件，不生成桌面安装包。

## 桌面 APP 构建

```bash
cd /Users/xusanye/Desktop/ai_manage/ai-auth-studio
npm run tauri:build
```

默认只生成 macOS `.app`，避免开发机上 DMG 打包依赖或签名流程影响基础构建。

构建产物：

```text
/Users/xusanye/Desktop/ai_manage/ai-auth-studio/src-tauri/target/release/bundle/macos/Super AI.app
```

如需单独打 DMG：

```bash
npm run tauri:build:dmg
```

## 本地预览生产包

```bash
cd /Users/xusanye/Desktop/ai_manage/ai-auth-studio
npm run preview -- --host 127.0.0.1
```

## 代码检查

```bash
cd /Users/xusanye/Desktop/ai_manage/ai-auth-studio
npm run lint
```

建议每次交付前执行：

```bash
npm run build
npm run lint
```

## 关键目录

```text
src/App.tsx              主界面
src/App.css              主界面样式
src/index.css            全局样式和主题变量
src/assets/logo.svg      Super AI 界面 Logo 源文件
src/lib/authParser.ts    Codex/Gemini JSON 解析原型
src-tauri/               Tauri 2 桌面端工程
src-tauri/icons/         Tauri 打包图标，由 logo.svg 生成
AGENTS.md                AI 开发协作说明
```

## 当前限制

- 已接入 Tauri 壳，但还没有实现 Rust 业务 commands。
- 暂不能直接读取 `~/.codex` 或 `~/.gemini`。
- 暂未实现 OAuth callback。
- 暂未实现账号写回/切换/注入。
- 当前文件导入使用浏览器 File API，仅用于前端原型验证。

## 命令速查

```bash
npm run dev          # 桌面 APP 开发，带前端热更新
npm run tauri:dev    # 同 npm run dev
npm run web:dev      # 仅网页预览，不打开 APP
npm run build        # 前端生产构建
npm run tauri:build  # 桌面 APP 打包，默认生成 .app
npm run tauri:build:dmg # macOS DMG 打包
npm run lint         # 代码检查
```
