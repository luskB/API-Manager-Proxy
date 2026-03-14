# Upload To GitHub

这份仓库已经按“公开上传”场景做过整理，但在真正上传前，仍建议你按下面步骤操作一次。

## 1. 使用干净目录

优先使用 `release/` 里整理好的公开源码目录，而不是直接把当前工作目录整个上传。

这样可以避免把以下内容一并带上去：

- 本地构建产物
- 临时备份
- 本地日志
- 个人开发缓存
- 任何 AppData 里的真实配置

## 2. 不要上传这些文件或目录

以下内容不要进入公开仓库：

- `C:\Users\<你的用户名>\AppData\Roaming\APIManager\`
- 任何 `apimanager_config.json`
- 任何 `model_cache.json`
- 任何 `proxy_stats.json`
- 任何 `security.db`
- 任何浏览器登录目录或 Cookie 数据
- `release/`
- `backup/`
- `node_modules/`
- `dist/`
- `target/`

## 3. 初始化 Git 仓库

在整理好的源码目录里执行：

```bash
git init
git branch -M main
git add .
git commit -m "Initial open source release"
```

## 4. 在 GitHub 创建新仓库

在 GitHub 新建一个空仓库，建议：

- 不勾选 `Add a README`
- 不勾选 `.gitignore`
- 不勾选 `License`

因为这些文件已经在本地目录里准备好了。

## 5. 关联远程仓库并推送

把下面的地址替换成你自己的 GitHub 仓库地址：

```bash
git remote add origin https://github.com/<your-name>/<your-repo>.git
git push -u origin main
```

如果你使用 SSH，也可以改成：

```bash
git remote add origin git@github.com:<your-name>/<your-repo>.git
git push -u origin main
```

## 6. 上传前最后检查

推送前建议再执行一次：

```bash
git status
git ls-files
```

重点确认：

- 没有任何配置 JSON
- 没有任何 API Key、Token、Cookie 相关文件
- 没有 `AppData` 内容
- 没有 `release`、`backup`、`node_modules`、`target` 等目录

## 7. 如果你想保留私人配置

请单独在本地备份：

- `C:\Users\<你的用户名>\AppData\Roaming\APIManager\`

这个目录只适合你自己私下保存，不适合公开上传。
