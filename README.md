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

打包前需要安装对应 Rust target，例如：

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
rustup target add x86_64-pc-windows-msvc aarch64-pc-windows-msvc
```

## 维护命令

检查代码：

```bash
npm run check
```

只跑 lint：

```bash
npm run lint
```

## API 服务状态

仓库当前只保留 `SuperAI` 页面和 API 服务卡片的 UI 壳子，不再内置可运行的本地 API 代理、sidecar 或 language server 打包链路。

如果后续要重新接回真实服务，请基于当前 UI 和设置结构重新实现，不要假定仓库里仍存在旧的代理或 sidecar 运行时。

## 远程升级

项目已接入 Tauri 2 updater。生产环境启动时会检测新版本；如果远程存在新版本，界面会显示强制升级弹窗，升级完成后自动重启 App。

当前 updater 地址：

```text
https://ai.talentisan.cn/SuperAI/latest.json
```

私钥文件：

```text
src-tauri/updater-private.key
```

该文件被 `.gitignore` 忽略，不要提交。`npm run build` 和 `npm run build:full` 会自动读取私钥并生成 updater 所需签名文件。发布远程升级前必须递增 `src-tauri/tauri.conf.json` 里的版本号，否则 Tauri updater 会认为没有新版本。

注意：

- GitHub Actions 里的 `rewrite-latest-json` 只会改写并回传 GitHub Release 附件里的 `latest.json`。
- 线上真正生效的更新源是 `https://ai.talentisan.cn/SuperAI/latest.json`。
- 如果没有把 release 里的 `latest.json` 和对应 `vX.Y.Z/` 安装包同步到 OSS/CDN，客户端仍然只会拿到旧版本。
- 因此“本地代码已经升到新版本，但客户端检测不到更新”时，先检查 OSS 上的 `latest.json` 版本和 `pub_date`，不要先怀疑前端弹窗或 updater 插件。
- 当前 workflow 只负责把 GitHub Release 里的 `latest.json` 改写成可直接上传 OSS 的版本；OSS 仍按你现有流程手动上传。

## 关键目录

```text
src/App.tsx              主界面
src/App.css              主界面样式
src/index.css            全局样式和主题变量
src/lib/authParser.ts    Codex/Gemini/SuperAI JSON 解析
src-tauri/               Tauri 2 桌面端工程
scripts/                 开发与打包脚本
AGENTS.md                AI 开发协作说明
```

## 命令速查

```bash
npm run dev                  # 用户版 App 开发启动
npm run dev:full             # 完整信息版 App 开发启动
npm run build                # 当前平台，公开版
npm run build:full           # 当前平台，完全版
npm run build:mac            # mac universal，公开版
npm run build:mac:full       # mac universal，完全版
npm run build:win:x64        # Windows x64，公开版
npm run build:win:x64:full   # Windows x64，完全版
npm run build:win:arm64      # Windows ARM64，公开版
npm run build:win:arm64:full # Windows ARM64，完全版
```
