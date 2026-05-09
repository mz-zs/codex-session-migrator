# Codex Session Migrator

一个用于迁移 Codex Desktop 会话的小工具。  
适合多台电脑使用 Codex 时，把某个项目下的会话，或者单条/多条会话，从一台电脑导出，再导入到另一台电脑。

## 功能

- 按 Codex 项目分组展示会话
- 支持勾选单条或多条会话导出
- 支持一键导出整个项目会话
- 导出为 `.codexpack` 迁移包
- 导入时可选择目标项目目录
- 导入前自动备份目标电脑的 Codex 状态文件
- 不迁移账号、token、配置和缓存
- 基于 Tauri，体积比 Electron 小很多

## 适用场景

比如你的 `AST` 项目里有多个 Codex 会话：

- 可以导出整个 `AST` 项目的全部会话
- 也可以只勾选其中需要的几条会话导出
- 到另一台电脑导入后，可以保留原项目路径，也可以指定新的项目目录

## 下载与运行

如果你已经有构建产物，可以直接运行：

```text
src-tauri/target/release/codex-session-migrator.exe
```

也可以使用安装包：

```text
src-tauri/target/release/bundle/nsis/Codex Session Migrator_0.1.0_x64-setup.exe
src-tauri/target/release/bundle/msi/Codex Session Migrator_0.1.0_x64_en-US.msi
```

如果是从源码运行：

```powershell
npm install
npm start
```

## 使用方法

### 导出会话

1. 打开工具
2. 确认顶部的 Codex 数据目录
   - 默认是当前用户的 `.codex` 目录
   - Windows 通常是 `C:\Users\<用户名>\.codex`
3. 在左侧选择项目
4. 在右侧勾选要迁移的会话
5. 点击 `导出选中` 或 `导出整个项目`
6. 保存生成的 `.codexpack` 文件

### 导入会话

1. 在另一台电脑打开工具
2. 切换到 `导入` 页面
3. 选择 `.codexpack` 迁移包
4. 如果项目路径和原电脑不同，选择目标项目目录
5. 勾选要导入的会话
6. 点击 `导入选中会话`
7. 关闭并重新打开 Codex Desktop，让它重新读取会话索引

## 导入时的目标项目目录

导入时可以留空，也可以手动选择目录。

留空时，工具会保留导出包里记录的原始项目路径。  
如果两台电脑的项目路径不同，建议手动选择目标项目目录。

例如：

```text
源电脑：D:\desktop\AST
目标电脑：E:\work\AST
```

导入时选择 `E:\work\AST`，导入后的会话就会归到目标电脑的这个项目下。

## 安全说明

工具只迁移这些内容：

- `state_5.sqlite` 里的会话索引
- `sessions/**/*.jsonl` 会话正文文件
- 少量和 thread 直接关联的状态表

工具不会迁移：

- `auth.json`
- `config.toml`
- 账号 token
- 模型配置
- 缓存文件
- 日志数据库

## 自动备份

导入前会自动备份目标电脑上的关键文件：

```text
<CodexHome>/backups_state/session-migrator-YYYYMMDD-HHMMSS/
```

备份内容包括：

- `state_5.sqlite`
- `state_5.sqlite-wal`
- `state_5.sqlite-shm`
- `session_index.jsonl`
- `.codex-global-state.json`

如果导入后发现不符合预期，可以从这个目录手动恢复。

## 开发

安装依赖：

```powershell
npm install
```

开发运行：

```powershell
npm start
```

构建：

```powershell
npm run build
```

构建产物：

```text
src-tauri/target/release/codex-session-migrator.exe
src-tauri/target/release/bundle/
```

## 技术栈

- Tauri 2
- Rust
- SQLite
- 原生 HTML/CSS/JavaScript

## 注意事项

- 建议导入前关闭 Codex Desktop
- 导入完成后重新打开 Codex Desktop
- 如果目标电脑没有对应项目目录，建议先创建项目目录，再导入时选择它
- `.codexpack` 里可能包含会话内容，请按私人数据处理
