## APIManagerProxy v1.5.0

本次发布主要聚焦在桌面使用体验、Hub 批量操作和统计展示优化。

### 新增与优化

- 监控页自动刷新状态现在会在本次应用运行期间保持，不会因为切换到仪表盘、Hub 或其它页面而自动关闭
- Hub 页面新增选中系统，支持：
  - 全选当前结果
  - 清除选中
  - 只刷新选中的站点余额与今日消耗
  - 只对选中的站点执行签到
- Token 统计页的时间窗口展示调整为“最新时间优先”，查看最近数据更直接
- README 已更新为当前产品名 `APIManagerProxy`，并同步整理了功能说明、接入方式和截图章节

### 已包含的近期稳定性修复

- 修复仪表盘顶部代理状态在启动初期误显示“已停止”的问题
- 修复 OpenCode CLI 同步路径与配置格式，适配当前 `~/.config/opencode/opencode.json`
- 完善 CLI 模型同步交互，支持同步全部、全部移除和二次确认
- 保持本地代理、Token 统计、Hub 刷新与批量操作之间的数据一致性

### 下载

| 平台 | 架构 | 文件 |
| --- | --- | --- |
| Windows | x64 | `apimanagerproxy.exe` |
| Windows | x64 | `APIManagerProxy_1.5.0_x64-setup.exe` |
| Windows | x64 | `APIManagerProxy_1.5.0_x64_en-US.msi` |
| macOS | Apple Silicon / Intel | GitHub Actions 自动构建产物 |
| Linux | x64 | GitHub Actions 自动构建产物 |
