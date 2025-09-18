# FRP Client Windows Service (frpc-service)

## 项目简介

`frpc-service` 是一个使用 Rust 语言开发的工具，旨在将一个或多个 [FRP Client](https://github.com/fatedier/frp) (`frpc`) 实例注册为一个统一的 Windows 服务。这使得 `frpc` 能够以静默、自动启动的方式在后台稳定运行，特别适合需要同时管理多个 FRP 连接的场景，摆脱了手动维护多个命令行的繁琐。

## 核心功能

- **自动化服务管理**：程序首次运行时，会自动将自身注册为名为 `FrpcService` 的 Windows 服务，并设置为开机自启。
- **多实例支持**：除默认的 `frpc.exe` 外，工具能自动发现并管理所有遵循 `frpc@<name>.exe` 命名规则的实例，并为每个实例加载对应的 `<name>.toml` 配置文件。这使得单服务下可以同时运行多个不同配置的 `frpc` 客户端。
- **智能服务清理**：当再次运行程序时，会提示您是否删除现有服务。确认后，程序会先停止服务并终止所有由其管理的 `frpc` 进程，然后干净地删除服务，防止进程残留。
- **统一日志记录**：所有 `frpc` 实例的标准输出和错误输出都会被捕获，并统一记录到日志文件中。日志会为每一条输出添加实例标识符（如 `[default]` 或 `[test]`），方便您区分和排查问题。日志同时会自动过滤掉 ANSI 转义字符，确保清晰可读。
- **图形化交互**：所有服务注册和删除的提示都通过 Windows 弹窗显示，提供了比命令行更友好的用户体验。

## 使用方法

### 准备工作

1. 从项目的 [Releases 页面](https://github.com/Colzry/frpc-service/releases)下载最新版本的 `frpc_service.exe`，或者自行编译此项目。
2. 将 `frpc_service.exe` 与 `frpc` 可执行文件和配置文件放入**同一目录**下。

**文件结构示例：**

```
/frpc/
│
├── frpc_service.exe  <-- 本工具
│
├── frpc.exe          <-- 默认实例的可执行文件
├── frpc.toml         <-- 默认实例的配置文件
│
├── frpc@test.exe     <-- 名为 "test" 的实例
├── test.toml         <-- "test" 实例的配置文件
│
├── frpc@another.exe  <-- 名为 "another" 的实例
└── another.toml      <-- "another" 实例的配置文件
```

> **提示**：`frpc@test.exe` 和 `frpc@another.exe` 通常只是 `frpc.exe` 的一个重命名副本。



### 注册服务

首次双击运行 `frpc_service.exe`，程序会自动执行以下操作：

1. 将自身注册为 `FrpcService` 服务。
2. 自动启动该服务。服务启动后，它会自动扫描并运行目录中所有符合规范的 `frpc` 实例。
3. 通过弹窗提示您服务已成功注册并运行。

### 删除服务

当 `FrpcService` 服务已存在时，再次运行 `frpc_service.exe`，程序会弹出确认窗口。

- 如果您选择**是**，程序将停止并删除该服务，并终止所有由它管理的 `frpc` 子进程。
- 如果您选择**否**，程序将直接退出，保留现有服务。

## 项目结构

- `src/main.rs`: 程序入口点，负责处理服务的注册、删除和运行模式的切换。
- `src/service.rs`: 包含了 `frpc-service` 运行在服务模式下的核心逻辑，负责发现并管理所有 `frpc` 子进程的生命周期。
- `src/frpc.rs`: 负责启动和终止单个 `frpc` 进程实例，并处理其标准输出。
- `src/logger.rs`: 日志模块，用于配置和管理日志输出格式。

## 编译与运行

确保您已安装 [Rust 环境](https://www.rust-lang.org/tools/install)。

```bash
cargo build --release
```

编译完成后，可执行文件目录为在 `target/release/frpc_service.exe` 。