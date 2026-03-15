# APIManagerProxy

一个面向本地使用场景的 AI API 聚合代理与中转站管理桌面工具。

APIManagerProxy 基于 `Tauri 2 + React + Rust` 构建，目标是把多个 AI 中转站、聚合站或兼容 OpenAI / Anthropic / Gemini 协议的站点统一接入到一个本地代理入口中，并提供余额查看、模型同步、日志监控、费用统计、CLI 配置同步、访问 Key 权限控制等能力。

它更适合这样的场景：

- 你手上有多个 AI API 中转站账号，希望统一查看余额、今日消耗、可用模型、API Key 和签到状态
- 你希望给 Cherry Studio、Chatbox、Claude Code、Codex CLI、OpenCode、Gemini CLI 等工具提供同一个本地代理入口
- 你希望给不同客户端或不同用户分配不同的本地访问 Key，并限制其可访问的站点和模型
- 你希望保留完整的请求日志、费用估算、Token 统计和请求重放能力，方便长期维护

## 当前能力

### 多站点账号管理

- 统一管理多个中转站账号
- 刷新站点 API Key，并在同站点多 Key 场景下切换当前使用 Key
- 查看余额、今日消耗、请求次数、模型列表与健康状态
- 支持从浏览器登录态和备份配置导入账号

### 本地统一代理

- 在本地启动统一代理端口
- 兼容 OpenAI / Anthropic / Gemini 风格接口
- 支持通过 `站点前缀::模型名` 精确路由到指定站点
- 转发到上游时自动去掉本地前缀，只发送真实模型名
- 支持本地访问 Key、模型白名单、站点白名单和限额控制

### Hub 管理

- 一键刷新全部站点余额与今日消耗
- 批量检测站点可用性
- 批量或单独执行签到
- 新增“选中站点”机制，支持只刷新或只签到选中的站点
- 支持自动刷新余额与今日消耗

### 监控与排障

- 记录请求时间、方法、路径、上游、模型、状态码、耗时、Token、费用
- 支持展开查看请求体 / 响应体
- 支持复制 cURL 和本地重放请求
- 监控页自动刷新状态在本次应用运行期间保持，不会因为切换页面而丢失

### 仪表盘与 Token 统计

- 仪表盘支持按小时 / 按天 / 按周切换请求趋势
- 支持费用、请求数、成功率、延迟等统计
- Token 统计页支持模型消耗分布、热门模型、站点消耗、时间窗口概览
- 时间窗口展示已调整为“当前时间优先”，更适合从最新数据开始查看

### CLI 配置同步

- 支持 Claude Code、Codex CLI、Gemini CLI、OpenCode、Droid
- 可从代理页直接选择需要同步到 CLI 的模型
- 支持多模型同步、同步全部、全部移除、二次确认
- OpenCode 已适配当前 `~/.config/opencode/opencode.json` 配置格式

## 界面截图

以下截图来自仓库内置演示资源，界面会随着版本继续迭代。

### 账号管理

![Accounts](./public/Introduction/1.png)

### Hub 资源管理

![Hub](./public/Introduction/2.png)

### CLI 配置同步

![CLI Sync](./public/Introduction/3.png)

### 本地代理与接入

![Proxy](./public/Introduction/4_1.png)

![Proxy Usage](./public/Introduction/4_2.png)

## 快速开始

### 1. 下载桌面版

当前仓库 Release 提供：

- Windows 便携版：`apimanagerproxy.exe`
- Windows 安装包：`APIManagerProxy_xxx_x64-setup.exe`
- Windows MSI：`APIManagerProxy_xxx_x64_en-US.msi`

### 2. 添加站点账号

启动应用后，先在 `账户` 页面添加你的站点账号，并确认这些信息可用：

- 站点地址
- 登录信息或访问令牌
- 当前可用 API Key
- 该站点实际可访问的模型列表

### 3. 在 Hub 页面检查站点状态

建议至少执行一次：

- 刷新余额
- 检测站点
- 检查可签到状态
- 如有需要，批量签到

### 4. 在代理页面启动本地代理

推荐流程：

1. 设定本地监听端口
2. 配置认证模式
3. 新建本地访问 Key
4. 为访问 Key 指定允许访问的站点和模型
5. 启动代理

### 5. 在客户端接入

以 OpenAI 兼容接口为例：

```bash
curl http://127.0.0.1:18090/v1/chat/completions \
  -H "Authorization: Bearer your-local-access-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5.2",
    "messages": [
      { "role": "user", "content": "Hello" }
    ]
  }'
```

如需定向到某个特定站点：

```bash
curl http://127.0.0.1:18090/v1/chat/completions \
  -H "Authorization: Bearer your-local-access-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "站点A::gpt-5.2",
    "messages": [
      { "role": "user", "content": "Hello" }
    ]
  }'
```

本地会把它解释为：

- 路由到 `站点A`
- 实际发给上游的模型名是 `gpt-5.2`

## 认证模式

支持以下代理认证模式：

- `off`：默认不强制要求本地访问 Key
- `strict`：所有代理请求都要求本地访问 Key
- `all_except_health`：除健康检查外全部要求鉴权
- `auto`：根据运行方式自动决定更合适的默认值

如果你准备长期使用“子 Key 限权”，建议优先使用 `strict` 或 `all_except_health`。

## 从源码运行

### 环境要求

- Node.js 20+
- pnpm 9+
- Rust stable
- Windows 下可用的 MSVC Rust 工具链

### 安装依赖

```bash
pnpm install
```

### 开发模式

```bash
pnpm tauri:dev
```

### 构建发布版

```bash
pnpm tauri:build
```

常见产物位置：

- `src-tauri/target/release/`
- `src-tauri/target/release/bundle/`
- `release/`

## 项目结构

```text
.
├─ src/                        # React 前端
│  ├─ components/              # 通用组件
│  ├─ hooks/                   # 自定义 Hooks
│  ├─ locales/                 # 多语言资源
│  ├─ pages/                   # 仪表盘 / 账户 / Hub / 代理 / Token统计 / 监控 / 设置
│  ├─ types/                   # 前端类型定义
│  └─ utils/                   # 前端工具函数
├─ src-tauri/                  # Rust 后端
│  ├─ src/
│  │  ├─ modules/              # 配置、Hub、桌面集成、安全数据库等
│  │  ├─ proxy/                # 转发、路由、统计、监控、CLI 同步等
│  │  ├─ commands.rs           # Tauri IPC 命令
│  │  └─ lib.rs                # 应用入口
│  └─ tauri.conf.json          # Tauri 打包配置
├─ public/Introduction/        # README 截图
└─ .github/workflows/          # CI / Release 工作流
```

## 技术栈

| 层级 | 技术 |
| --- | --- |
| 桌面框架 | Tauri 2 |
| 前端 | React 19 + TypeScript + Vite |
| 样式 | Tailwind CSS + DaisyUI |
| 后端 | Rust |
| 代理服务 | Axum + reqwest |
| 数据存储 | JSON + SQLite |

## 使用建议

### 为高成本模型单独分配访问 Key

如果你有高价值或高价格模型，推荐：

- 为其创建独立访问 Key
- 只开放必要的站点和模型
- 开启日志与统计，方便长期观察

### 把“轻量刷新”和“全量检测”分开使用

- 日常看余额、今日消耗时，用 Hub 页面刷新即可
- 需要重新抓取模型、定价、签到状态时，再执行检测全部

### 通过 CLI 同步减少手工维护

如果你同时使用多个 Agent / CLI 工具，推荐直接在代理页统一选择模型，再同步给 OpenCode、Claude Code、Codex CLI 等工具，避免每个工具分别改配置。

## 已知说明

- 不同中转站后台实现差异较大，部分字段名称、返回格式和功能支持程度并不完全一致
- 某些站点不会回传真实明文 API Key，需要你在站点创建时自行保存
- 模型价格、余额、今日消耗等统计依赖站点接口质量，个别站点可能会返回不完整数据

## 致谢

项目在设计与开发过程中参考了以下开源项目的思路与实现：

- [zhalice2011/api-manager](https://github.com/zhalice2011/api-manager)

同时也感谢所有兼容 OpenAI / Anthropic / Gemini 协议的开源生态，为本地聚合代理与客户端接入提供了大量实践基础。

## License

[MIT](./LICENSE)
