# Rust 全局配置与项目静态 CRT

## 当前配置检查

2026-10-07 检查本机配置：

- `%USERPROFILE%\.cargo\config.toml` 将 crates.io 替换为 `rsproxy-sparse`，地址为 `sparse+https://rsproxy.cn/index/`；没有全局 `build.target`、`rustflags` 或 `target-dir`。
- 当前进程没有 `CARGO_HOME`、`RUSTFLAGS`、`CARGO_ENCODED_RUSTFLAGS` 覆盖。
- `RUSTUP_DIST_SERVER=https://rsproxy.cn`，`RUSTUP_UPDATE_ROOT=https://rsproxy.cn/rustup`：影响工具链下载，不设置应用链接方式。
- rustup 默认工具链为 `stable-x86_64-pc-windows-msvc`；实际 rustc 为 `1.99.0 (b940084d7 2026-09-28)`。
- 修改前运行 `build.bat debug` 成功，耗时 1m34s。未观察到当前全局配置造成编译失败；没有全局配置旧快照，因此不推断用户具体删除或新增了哪些设置。

镜像替换影响依赖下载来源与缓存命中，不改变 Cargo.lock 中 crates.io 的逻辑来源，也不等于启用静态 CRT。Rust 工具链更新、目标或编译参数变化可能使旧构建缓存无法复用。

## 项目配置

项目原来已在通用 `[build].rustflags` 中设置 `+crt-static`，并非完全没有静态 CRT。现在不设置默认 target，按当前 host 编译，仅限制 CRT 参数适用的平台：

```toml
[target.'cfg(all(target_os = "windows", target_env = "msvc"))']
rustflags = [
    "-C", "target-feature=+crt-static",
    "-C", "link-arg=/NODEFAULTLIB:ucrt",
    "-C", "link-arg=libucrt.lib",
]
```

配置位于 `src-tauri/.cargo/config.toml`。本地 `build.bat` 与 CI 均在 `src-tauri` 中调用 Cargo，因此会加载该文件。没有外部 target 覆盖时，构建产物位于 `src-tauri/target/<profile>/`。

当前不区分显式 target 与 host，构建脚本和过程宏也可能接收匹配 host 的 rustflags。显式 `--target` 仍可用于交叉编译；非 Windows MSVC 不匹配 CRT 配置。调用者的全局 `build.target`、`RUSTFLAGS` 或 `CARGO_ENCODED_RUSTFLAGS` 可以改变构建行为，验证前应检查这些覆盖。

静态 CRT 不表示整个程序没有 DLL 依赖：Windows 系统 DLL 和 WebView2 仍有各自的运行环境要求。

### 实际发现的动态 UCRT

原配置和仅调整 target 的构建均成功，但 EXE 导入表仍包含 10 个 `api-ms-win-crt-*` DLL。同工具链最小 Rust 程序使用 `+crt-static` 时没有这些导入；已检查的 SQLite、AWS-LC 和 WebView2 loader 原生库指令使用 `LIBCMT`。最终链接映射确认 `mainCRTStartup` 来自 `libcmt:exe_main.obj`，而 `malloc`、`pow`、`strlen` 等符号来自动态 `ucrt` 导入库。

因此不能把“已设置 +crt-static”当成“整个应用已静态链接 CRT”。新增 `/NODEFAULTLIB:ucrt` 排除动态 UCRT，显式选择 `libucrt.lib`。最小验证仅在最终链接追加这两个参数：构建成功，10 个 CRT DLL 导入全部消失。随后按最终项目配置重新构建和检查两种 profile。

## 验证方法

1. 运行 `build.bat all`，确认 debug 和 release 均成功。
2. 在 Visual Studio 开发者命令行运行 `dumpbin /dependents bin\debug\niuma-timer.exe` 和 `dumpbin /dependents bin\release\niuma-timer.exe`。
3. 检查导入表不包含 `VCRUNTIME*.dll`、`MSVCP*.dll`、`ucrtbase.dll`、`api-ms-win-crt-*.dll` 等动态 CRT 导入。仍有 Windows 系统 DLL 属于正常情况。
4. 运行 `build.bat test`，验证 Clippy、Rust 测试和其他项目门禁。

该检查针对 EXE 的直接导入，不代表已在未安装 VC++ 运行库的全新机器上做过运行验收。

## 显式 target 配置阶段的验证结果

- 该阶段配置执行 `build.bat all`：退出码 0，debug 与 release 均编译成功。
- `dumpbin /dependents` 检查最终 `bin/debug/niuma-timer.exe` 和 `bin/release/niuma-timer.exe`，两者均无 `VCRUNTIME*`、`MSVCP*`、`MSVCR*`、`ucrtbase.dll` 或 `api-ms-win-crt-*` 直接导入。分别保留 23 和 22 项 Windows 系统 DLL 导入。
- `build.bat test`：退出码 0；fmt、Clippy、在线依赖安全审计、Rust 261 项、前端 38 个脚本、Python 29+11+4 项均通过。
- 全局 Rust 配置未修改，Cargo.lock 和应用版本未修改。
- 未启动真实应用，未在干净 Windows 虚拟机做运行验收，未生成新安装包。验证针对本次 debug/release EXE。

## 按当前 host 构建

按用户要求移除项目 `[build].target`。同时修正 `build.bat` 的产物复制顺序：优先使用 `target/<profile>/` 的默认 host 产物，缺失时再查旧的 host triple 目录。安装包、便携 EXE 和签名使用同样的默认目录优先顺序。这样移除 target 后，不会因保留旧目录而误复制旧 EXE。

`scripts/test_build_env.js` 在隔离目录构造两套产物：先保留旧 triple 目录，再生成默认 host 目录的新产物。修复前 debug/release 两项复制断言失败，修复后 32 项断言全部通过，包括安装包、便携 EXE 和签名。

移除 target 后重新运行 `build.bat all`：退出码 0，debug 用时 1m14s、release 用时 3m17s。重新运行 `build.bat test`：退出码 0，Rust 261 项、前端 38 个脚本和 Python 29+11+4 项全部通过，fmt、Clippy 和在线依赖审计通过。

重新检查默认 host 的 `bin/debug` 和 `bin/release` EXE 导入表：分别有 23 和 22 项 Windows 系统 DLL 导入，均无动态 CRT 导入。保留静态 CRT 配置有效。

参考：[Cargo 配置优先级与目标参数](https://doc.rust-lang.org/stable/cargo/reference/config.html)、[Rust 静态和动态 C 运行库](https://doc.rust-lang.org/stable/reference/linkage.html#static-and-dynamic-c-runtimes)。
