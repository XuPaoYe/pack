# Super AI Operations

运维/部署说明。产品目标、实现约束和 AI 接手规则见 `AGENTS.md`。

## 环境要求

- Node.js 22+
- npm 10+
- Rust stable / Cargo
- Tauri 2 相关系统依赖
- macOS / Windows 打包签名环境

## 安装依赖

```bash
npm install
```

## 开发启动

```bash
npm run dev
```

默认开发启动就是用户版：SuperAI 账号卡片的标题、名称、账号和账号等级会脱敏显示；有效期显示传入/解析到的到期时间；SuperAI 只显示日限，不显示周限和日限刷新时间；SuperAI 导出只输出账号、密码、到期时间三项加密字段；日志和提示不暴露 SuperAI 账号信息。Codex / Gemini 不隐藏。

自己调试完整信息时使用：

```bash
npm run dev:full
```

## 打包

```bash
npm run tauri:build
```

默认打包用户版，产物在：

```text
src-tauri/target/release/bundle/
```

自己打完整信息版：

```bash
npm run tauri:build:full
```

单独打 macOS DMG：

```bash
npm run tauri:build:dmg
```

完整信息版 DMG：

```bash
npm run tauri:build:dmg:full
```

## API Sidecar

第一次开发或升级 vendor 后先构建 sidecar：

```bash
npm run build:sidecar
```

需要 `bun >= 1.3`，并提前安装好对应的本地 IDE 客户端，或通过环境变量指向 language server 二进制（详见 `scripts/build-sidecar.sh`）。

## 检查

```bash
npm run check
```

或单独运行：

```bash
npm run lint
```

## 远程升级

项目已接入 Tauri 2 updater。生产环境启动时会检测新版本；如果远程存在新版本，界面会显示强制升级弹窗，升级完成后自动重启 App。

当前 updater 地址：

```text
https://ai.talentisan.cn/super-ai/latest.json
```

私钥文件：

```text
src-tauri/updater-private.key
```

该文件被 `.gitignore` 忽略，不要提交。`npm run tauri:build` 和 `npm run tauri:build:dmg` 会自动读取私钥并生成 updater 所需签名文件。

## 关键目录

```text
src/App.tsx              主界面
src/App.css              主界面样式
src/index.css            全局样式和主题变量
src/lib/authParser.ts    Codex/Gemini/SuperAI JSON 解析
src-tauri/               Tauri 2 桌面端工程
scripts/                 开发、打包和 sidecar 脚本
AGENTS.md                AI 开发协作说明
```

## 命令速查

```bash
npm run dev                  # 用户版 App 开发启动
npm run dev:full             # 完整信息版 App 开发启动
npm run tauri:build          # 用户版 App 打包
npm run tauri:build:full     # 完整信息版 App 打包
npm run tauri:build:dmg      # 用户版 macOS DMG
npm run build:sidecar        # 构建本地 API sidecar
npm run check                # 构建 App 前端资源并 lint
npm run lint                 # 代码检查
```
