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

## App 开发启动

```bash
npm run dev
```

默认启动公开版：SuperAI 账号卡片的标题、名称、账号和账号等级会脱敏显示；有效期显示批量密钥里的到期时间；SuperAI 只显示日限，不显示周限和日限刷新时间；SuperAI 导出只输出批量密钥；日志和提示不暴露 SuperAI 账号信息。Codex / Gemini 不隐藏。

调试完全版时使用：

```bash
npm run dev:full
```

完全版不脱敏，SuperAI 账号卡片有效期显示账号信息里的时间，不显示批量密钥里的到期时间。

## App 打包

```bash
npm run build
```

默认打包公开版 App，产物在：

```text
src-tauri/target/release/bundle/
```

打包完全版 App：

```bash
npm run build:full
```

`build` / `build:full` 内部调用 Tauri CLI 完成桌面 App 打包。项目只对外保留 App 命令，不再把 `tauri:*` 作为日常入口。

## 多架构 App 打包

常用电脑需要覆盖这些目标：

| 电脑 | 目标 | 公开版命令 | 完全版命令 |
|---|---|---|---|
| Mac Apple Silicon / Intel | `universal-apple-darwin` | `npm run build:mac` | `npm run build:mac:full` |
| Windows x64 | `x86_64-pc-windows-msvc` | `npm run build:win:x64` | `npm run build:win:x64:full` |
| Windows ARM64 | `aarch64-pc-windows-msvc` | `npm run build:win:arm64` | `npm run build:win:arm64:full` |

mac universal 包需要先准备 universal sidecar：

```bash
TARGET=universal-apple-darwin npm run build:sidecar
```

Windows x64 / ARM64 包需要分别准备对应 sidecar，并建议在对应 Windows 构建机或 CI 上打包：

```bash
TARGET=windows-x64 npm run build:sidecar
TARGET=windows-arm64 npm run build:sidecar
```

Windows ARM64 还依赖对应架构的 SuperAI runtime / language server。如果本机无法自动找到，手动设置 `WINDSURF_LS_ARM64_PATH` 指向 ARM64 版本二进制。

打包前需要安装对应 Rust target，例如：

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
rustup target add x86_64-pc-windows-msvc aarch64-pc-windows-msvc
```

## 维护命令

第一次开发或升级 vendor 后先构建 sidecar：

```bash
npm run build:sidecar
```

需要 `bun >= 1.3`，并提前安装好对应的本地 IDE 客户端，或通过环境变量指向 language server 二进制（详见 `scripts/build-sidecar.sh`）。这是维护命令，不是 App 打包入口。

检查代码：

```bash
npm run check
```

只跑 lint：

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

该文件被 `.gitignore` 忽略，不要提交。`npm run build` 和 `npm run build:full` 会自动读取私钥并生成 updater 所需签名文件。

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
npm run build                # 用户版 App 打包
npm run build:full           # 完整信息版 App 打包
npm run build:mac            # 用户版 mac universal App 打包
npm run build:win:x64        # 用户版 Windows x64 App 打包
npm run build:win:arm64      # 用户版 Windows ARM64 App 打包
```
