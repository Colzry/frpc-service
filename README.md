# FRP Client Windows Service (frpc-service)

## 项目简介

`frpc-service` 是一个使用 Rust 语言开发的工具，旨在将 [FRP Client](https://github.com/fatedier/frp) (`frpc.exe`) 注册为 Windows 服务。这使得 `frpc` 能够以静默、自动启动的方式在后台稳定运行，摆脱了手动在命令行中启动和管理的繁琐。

## 核心功能

- **自动化服务管理**：程序首次运行时，会自动将 `frpc.exe` 注册为名为 `FrpcService` 的 Windows 服务，并设置为开机自启。
- **智能服务清理**：当再次运行程序时，会提示您是否删除现有服务。确认后，程序会先停止服务并终止所有相关的 `frpc.exe` 进程，然后干净地删除服务，防止进程残留。
- **统一日志记录**：`frpc` 进程的所有标准输出和错误输出都会被捕获，并统一记录到应用的日志文件中，同时自动过滤掉烦人的 ANSI 转义字符，确保日志清晰可读。
- **图形化交互**：所有服务注册和删除的提示都通过 Windows 弹窗显示，提供了比命令行更友好的用户体验。

## 使用方法

### 准备工作

1. 下载并编译此项目，生成 `frpc_service.exe`。
2. 将 `frpc_service.exe`、`frpc.exe` 和您的 `frpc.toml` 配置文件放置在**同一目录**下。

### 注册服务

首次运行 `frpc_service.exe`，程序会自动执行以下操作：

1. 将自身注册为 `FrpcService` 服务。
2. 自动启动该服务。
3. 通过弹窗提示您服务已成功注册并运行。

### 删除服务

当 `FrpcService` 服务已存在时，再次运行 `frpc_service.exe`，程序会弹出确认窗口。

- 如果您选择**是**，程序将停止并删除该服务，并终止所有 `frpc.exe` 进程。
- 如果您选择**否**，程序将直接退出，保留现有服务。

## 项目结构

- `src/main.rs`: 程序入口点，负责处理服务的注册、删除和运行模式的切换。
- `src/service.rs`: 包含了 `frpc-service` 运行在服务模式下的核心逻辑，处理服务的生命周期管理。
- `src/frpc.rs`: 负责启动和终止 `frpc` 进程，并处理其标准输出。
- `src/logger.rs`: 日志模块，用于配置和管理日志输出格式。

## 编译与运行

使用 Cargo 工具进行编译：

```
cargo build --release
```

编译完成后，可执行文件将在 `target/release/frpc_service.exe` 目录下。