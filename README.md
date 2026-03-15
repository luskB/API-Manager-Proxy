# APIManagerProxy

本地 AI API 聚合管理与统一代理工具。

APIManagerProxy 是一个基于 Tauri 2、React 和 Rust 构建的桌面应用，目标是把分散在多个 AI API 中转站、聚合站、兼容 OpenAI/Anthropic/Gemini 协议的站点资源统一管理起来，并通过本地端口提供一个稳定、可观测、可控的统一代理入口。

它适合这样的使用场景：

- 你同时维护多个 AI API 站点账号，希望统一查看额度、模型、Key、签到、消耗等信息
- 你希望给 Cherry Studio、Chatbox、Claude Code、Codex CLI、Gemini CLI 等工具提供一个本地统一入口
- 你希望对不同访问 Key 做模型/站点限制，方便多人共用或分组管理
- 你希望保留请求日志、监控、统计和重放能力，便于排查中转站异常

## 项目特点

### 1. 多站点账号管理

- 支持批量管理多个 AI API 站点账号
- 支持刷新额度、今日消耗、可用模型、签到状态等信息
- 支持手动维护站点 API Key，并在一个站点存在多个 Key 时切换当前使用的 Key
- 支持从浏览器存储、备份文件等来源导入配置

### 2. 本地统一代理

- 在本地启动统一代理端口
- 兼容 OpenAI、Anthropic、Gemini 等常见协议
- 支持通过 `站点名::模型名` 的方式定向请求指定站点
- 转发到上游时只发送真实模型名，避免把本地前缀错误传给上游
- 支持子 API Key、模型白名单、站点白名单等访问控制

### 3. 监控与问题排查

- 实时记录请求日志
- 支持查看请求体、响应体、状态码、耗时、Token、费用等信息
- 支持重放请求，快速复现问题
- 支持在代理问题、模型路由问题、额度异常问题出现时做针对性排查

### 4. 仪表盘与统计

- 提供请求量、Token 消耗、费用、成功率等统计视图
- 支持按小时、按日、按周切换统计粒度
- 支持查看访问 Key 维度的使用情况
- 支持监控模型、上游、账号等维度的数据

### 5. Hub 管理能力

- 支持获取站点余额和今日消耗
- 支持一键检测全部账号
- 支持自动签到
- 支持轻量刷新余额与今日消耗
- 支持设置自动刷新及刷新间隔

## 截图

> 下列截图来自项目内置演示资源，实际界面会随着版本演进持续调整。

### 账号管理

![Accounts](./public/Introduction/1.png)

### Hub 检测与资源管理

![Hub](./public/Introduction/2.png)

### CLI 配置同步

![CLI Sync](./public/Introduction/3.png)

### 本地代理使用

![Proxy](./public/Introduction/4_1.png)

![Proxy Usage](./public/Introduction/4_2.png)

## 适用对象

这个项目更适合以下用户：

- 手里有多个 AI API 中转站账号的重度使用者
- 需要给本地客户端统一接入多个上游的个人用户
- 想给不同人分配不同访问 Key 权限的小团队
- 希望把多个中转站的模型能力整合到一个本地代理入口中的开发者

如果你只使用单一官方 API，或者不需要本地代理和监控能力，这个项目可能会显得偏重。

## 支持能力概览

### 兼容协议

- OpenAI Compatible API
- Anthropic Messages API
- Gemini Compatible API

### 支持的常见站点类型

项目针对常见中转站面板和兼容实现做了适配，通常包括但不限于：

- New API
- One API
- Veloera
- OneHub
- DoneHub
- Sub2API
- AnyRouter
- VoAPI
- Super API

不同站点实现可能存在定制差异，实际可用能力以对应站点接口返回结果为准。

## 核心设计思路

APIManagerProxy 并不是简单把多个 Key 拼在一起，而是把“账号管理”和“代理转发”拆成两层：

1. 账号层：维护站点、账号、余额、今日消耗、模型、当前启用 API Key 等基础信息。
2. 代理层：面向本地应用统一暴露 OpenAI/Anthropic/Gemini 风格接口，并根据请求中的模型、访问 Key 权限、站点前缀等规则进行转发。

一个非常重要的约定是：

- `站点名::模型名` 只用于本地路由
- 真正发给上游时只会保留 `模型名`

例如：

```json
{
  "model": "站点A::gpt-5.2"
}
```

在本地会被解释为：

- 路由到 `站点A`
- 实际请求上游模型 `gpt-5.2`

## 功能模块

### Accounts

- 管理各个站点账号
- 刷新站点 API Key
- 切换站点当前启用的 Key
- 编辑站点、账号、代理设置等基础信息

### Hub

- 查看余额
- 查看今日消耗
- 检测模型与定价
- 自动签到
- 自动刷新余额与消耗

### Proxy

- 启动本地代理服务
- 配置监听端口、认证方式、访问 Key
- 为访问 Key 分配可访问站点与模型
- 用于接入 Cherry Studio、Chatbox、CLI 等本地应用

### Monitor

- 查看请求记录
- 观察状态码、耗时、Token、费用
- 复制请求
- 重放请求

### Dashboard

- 按小时 / 每日 / 每周统计请求趋势
- 查看模型、站点、访问 Key 的使用情况
- 汇总本地代理的消耗与成功率

### Settings

- 通用设置
- 多语言
- 主题切换
- CLI 配置同步

## 快速开始

### 1. 下载并安装

你可以直接使用打包好的桌面版本：

- Windows: `.exe` 或 `.msi`

也可以自行从源码构建，见下方“从源码运行 / 构建”。

### 2. 添加账号

打开应用后，先在 `Accounts` 页面添加你的站点账号。

建议至少确认以下信息可用：

- 站点地址
- 访问令牌或登录信息
- 当前可用 API Key
- 站点可用模型列表

### 3. 在 Hub 页面检测余额和可用信息

建议至少做一次：

- 刷新余额
- 刷新今日消耗
- 检测模型
- 自动签到测试

### 4. 在 Proxy 页面启动本地代理

推荐流程：

1. 选择本地监听端口
2. 设置认证模式
3. 新建一个本地访问 Key
4. 给这个访问 Key 指定允许访问的站点和模型
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

如果你需要强制定向到某个站点：

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

## 访问控制

APIManagerProxy 支持为“本地访问 Key”配置权限，而不是把所有上游能力直接暴露给所有客户端。

你可以为某个访问 Key 限制：

- 可访问站点
- 可访问模型
- 使用范围

这对于下列场景很有帮助：

- 给不同客户端分配不同权限
- 将测试模型与正式模型分离
- 限制某个 Key 只能访问少数几个高价值模型
- 团队内部按站点或按用途分组

## 认证模式

项目支持多种代理认证模式：

- `off`：不强制要求所有请求带本地访问 Key
- `strict`：所有代理请求都必须带本地访问 Key
- `all_except_health`：除了健康检查接口外都需要本地访问 Key
- `auto`：根据运行方式自动选择更合适的默认行为

如果你计划长期使用“子 API Key 限权”功能，建议优先使用 `strict` 或 `all_except_health`。

## 从源码运行

### 环境要求

- Node.js 20+
- pnpm 9+
- Rust stable
- Windows 需要可用的 MSVC Rust 工具链

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

构建完成后，常见产物位于：

- `src-tauri/target/release/`
- `src-tauri/target/release/bundle/`

## Headless 模式

除了桌面界面，你也可以把它当作本地或轻量服务器代理运行。

示例：

```bash
ABV_HEADLESS=1 PORT=8080 ABV_API_KEY=your-key ABV_AUTH_MODE=strict ./apimanagerproxy
```

常见环境变量如下：

| 变量名 | 说明 |
| --- | --- |
| `ABV_HEADLESS` | 设为 `1` 启用无窗口模式 |
| `PORT` | 本地代理监听端口 |
| `ABV_API_KEY` | 默认访问 Key |
| `ABV_AUTH_MODE` | 鉴权模式，可选 `off` / `strict` / `all_except_health` / `auto` |
| `ABV_BIND_LOCAL_ONLY` | 设为 `true` 时仅监听本机 |

## 项目结构

```text
.
├─ src/                      # React 前端
│  ├─ pages/                 # 主要页面：Dashboard / Accounts / Hub / Proxy / Monitor / Settings
│  ├─ components/            # 通用组件
│  ├─ types/                 # 前端类型定义
│  ├─ hooks/                 # 自定义 Hooks
│  ├─ locales/               # 多语言资源
│  └─ utils/                 # 工具函数
├─ src-tauri/                # Rust 后端
│  ├─ src/
│  │  ├─ proxy/              # 本地代理、路由、限流、监控、模型缓存、价格缓存
│  │  ├─ modules/            # 配置、Hub 交互、登录、备份、安全数据库等
│  │  ├─ commands.rs         # Tauri IPC 命令
│  │  ├─ models.rs           # 核心数据结构
│  │  └─ lib.rs              # Tauri 应用入口
│  ├─ icons/                 # 打包图标资源
│  └─ tauri.conf.json        # Tauri 打包配置
├─ public/Introduction/      # README 演示截图
└─ .github/workflows/        # CI / Release 工作流
```

## 技术栈

| 层 | 技术 |
| --- | --- |
| 桌面应用框架 | Tauri 2 |
| 前端 | React 19 + TypeScript + Vite |
| 样式 | Tailwind CSS + DaisyUI |
| 后端 | Rust |
| 代理服务 | Axum + reqwest |
| 数据持久化 | JSON + SQLite |

## 使用建议

### 1. 优先把“站点前缀”当成本地路由标记

推荐用法：

- `gpt-5.2`
- `站点A::gpt-5.2`

不推荐把前缀模型名直接录入到上游站点里，因为上游通常只认识真实模型名。

### 2. 对高价值模型单独分配访问 Key

如果你有高成本模型，建议：

- 单独建一个访问 Key
- 只允许少数模型
- 开启代理日志与统计

### 3. 把日常“刷新”与“检测全部”分开使用

- 日常查看余额和今日消耗时，用轻量刷新即可
- 需要重新抓取模型、价格、签到等信息时，再使用检测全部

## 已知说明

- 不同站点的 API 面板实现差异很大，少量站点可能存在定制字段
- 某些站点不会返回完整明文 API Key，需要你在站点创建时手动保存
- 模型价格、余额、今日消耗等信息依赖站点接口质量，个别站点可能返回不一致数据

## Roadmap

后续可以继续演进的方向包括：

- 更细粒度的访问 Key 配额控制
- 更丰富的图表统计
- 更多站点面板的自动识别
- 更完整的导入 / 导出能力
- 更详细的异常诊断与修复建议

## 致谢

本项目在开发过程中参考了以下开源项目与思路：

- [zhalice2011/api-manager](https://github.com/zhalice2011/api-manager)

这里的“参考”主要是指产品方向、部分交互思路和相关领域实践。当前仓库已经根据实际使用场景进行了较多扩展、修复与定制化调整。

同时也感谢所有提供 OpenAI 兼容、Anthropic 兼容、Gemini 兼容接口实践经验的社区项目与使用者。

## 免责声明

请在遵守你所使用站点、模型服务商和相关法律法规的前提下使用本项目。

本项目仅提供本地管理、转发与可观测能力，不对第三方站点的稳定性、可用性、合规性和计费准确性作任何保证。由于第三方站点接口策略可能随时变化，请自行评估使用风险。

## License

本项目采用 [MIT License](./LICENSE) 开源。
