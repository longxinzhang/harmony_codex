# 终端 SIGSYS 诊断补丁与构建说明

这是针对真机 `pwd` 返回 159 的诊断补丁，尚未确定或修复触发 SIGSYS 的系统调用，不能作为已通过真机验证的安装包。

## 源码基线与证据

基线是 `codex/harmony-native` 分支的提交 `48942e33bfd17778d41af9281e01fcaf52c18158`。
相较 g913，该基线包含挂载修复和原生目录探针生命周期修复，子进程启动模块在这段提交区间没有改变。

已取得的设备结果：

- g913 使用全权限启动参数时，工具记录 `/bin/sh -lc pwd`，返回 159、空输出。
- `helper-bootstrap-wait` 和 `pipe-wait` 都为 `exit_code=None signal=Some(31) raw_wait_status=159 core_dumped=true`。
- 用户确认系统终端直接执行 `/bin/sh -c pwd` 与 `/bin/sh -lc pwd`，退出码均为 0。
- 主进程退出 0，不能代替命令工具验收。

旧日志中的 `pid` 是记录日志的父进程 PID，不是被等待子进程的 PID。
辅助程序失败时，其独立 stderr 管道会随句柄丢弃；因此缺少辅助程序内部阶段日志，不能证明它没有进入辅助入口。

## 改动

1. 启动和等待日志增加 `child_pid`，在等待前保留它，以免回收后 PID 消失。
2. 增加辅助程序启动、pipe/PTY 回退及兼容后端启动阶段。
3. 在兼容后端的 `pre_exec` 中记录会话、父进程退出信号、描述符清理和设置完成阶段。开关在父进程读取；子进程只使用栈内缓冲区、`getpid` 和 `write`，避免在 fork 后使用环境查询、格式化分配或标准输出锁。
4. 辅助程序未发回执行确认时，最多读取 8192 字节的 stderr，最多等待 500 ms；只转发匹配真实子进程 PID 的固定阶段标签，不复制任意 stderr 内容或额外字段。这项收集只在明确启用进程诊断时执行。
5. 增加真实子进程 PID、关闭诊断时的输出、阶段过滤，以及失败辅助程序转发的回归用例。

该补丁没有加入 SIGSYS 信号处理器，不能直接提供 `si_syscall`。需要先观察保留下来的最后阶段，再取得匹配 child PID 的系统故障记录，或做进一步定向插桩。

## 当前验证状态

- 六个修改的 Rust 文件已通过 tree-sitter Rust 语法解析。
- `git diff --check` 已通过。
- 已核对等待日志调用点与修改后的函数签名。
- 构建脚本的 help 已核对，以下参数均来自仓库现有入口。
- 当前执行环境没有 Rust 工具链与 DevEco 原生 SDK。Rust 类型检查、rustfmt、单元/集成测试、OHOS 编译、签名和真机测试均未运行。语法解析不能替代这些检查。
- 新增回归测试已经写入源码，但不能记为“测试通过”。

## 从自己的仓库获取修改

修改放在 `longxinzhang/harmony_codex` 的独立分支 `codex/harmony-sigsys-diagnostics`。
在已配置构建环境的 Mac 上，直接拉取这个分支，无需再次应用补丁：

```sh
git clone --single-branch --branch codex/harmony-sigsys-diagnostics \
  https://github.com/longxinzhang/harmony_codex.git harmony-codex-sigsys
cd harmony-codex-sigsys
git log -1 --oneline
```

可以在这个分支继续修改和提交。仓库默认的 `main` 分支不是本次鸿蒙候选源码，构建前应确认当前分支。

## 使用补丁文件获取修改

使用一个独立的源码目录，保留现有设备安装：

```sh
git clone --filter=blob:none --branch codex/harmony-native \
  https://github.com/uluckyXH/codex.git codex-harmony-sigsys
cd codex-harmony-sigsys
git switch -c local/harmony-sigsys-diagnostics \
  48942e33bfd17778d41af9281e01fcaf52c18158
git am /补丁的实际路径/鸿蒙Codex-SIGSYS诊断补丁.patch
```

补丁提交本身包含本页。编译宿主需要 Rust 1.95.0、OHOS Rust 目标、完整 DevEco 原生 SDK、Python 3.12 以上，以及与签名工具兼容的 Java。
现有详细配置见[Mac 构建环境](苹果电脑构建环境与外置盘配置.md)和[构建签名流水线](原生资源构建签名与安装流水线.md)。

## 构建前检查

在应用补丁后的仓库中：

```sh
cd codex-rs
cargo fmt -p codex-utils-pty -- --check
cargo test -p codex-utils-pty --lib
cd ..
python3 scripts/build_harmony.py check \
  --sdk "$OHOS_NDK_HOME" --package codex-utils-pty
```

`OHOS_NDK_HOME` 指向实际 SDK 的 native 目录。先通过宿主测试和 OHOS 类型检查，再制作签名候选包。任何失败都应保留原始输出，不能用语法解析结果替代。

## 制作签名候选包

以下环境变量应来自本机实际路径：`OHOS_NDK_HOME`、`HARMONY_JAVA`。Rust 和 Cargo 也必须已配置好。
在仓库根目录执行，批次输出采用全新目录：

```sh
harmony_sigsys_batch="$PWD/.harmony-build/SIGSYS诊断-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$harmony_sigsys_batch"

python3 scripts/build_harmony_distribution.py helpers \
  --sdk "$OHOS_NDK_HOME" --java "$HARMONY_JAVA" \
  --source-cache "$PWD/.harmony-build/源包缓存" \
  --output-dir "$harmony_sigsys_batch/原生资源"

python3 scripts/build_harmony_distribution.py package \
  --sdk "$OHOS_NDK_HOME" --java "$HARMONY_JAVA" \
  --runtime-base /data/storage/el2/base/files \
  --helpers-dir "$harmony_sigsys_batch/原生资源" \
  --output-dir "$harmony_sigsys_batch/发布产物"
```

应使用仓库流水线保持辅助工具签名、摘要编入 CLI、CLI 签名和清单生成的顺序。候选包应标注未真机验证，并记录补丁提交与 SDK 身份。

## 真机复测

按新候选包安装说明装入独立新目录，核对 `--version` 为补丁构建，再明确开启：

```text
CODEX_HARMONY_PROCESS_DIAGNOSTICS=1
```

用绝对路径启动新候选，只调用一次终端工具执行 `pwd`，保留主进程 stderr 和工具事件。
重点查找：

- `helper-native-spawned` 与对应 `helper-bootstrap-wait` 的 child PID。
- 从失败辅助程序中保留下来的最后阶段。
- `pipe-compatibility-fallback`。
- `compat-child-entry` 到 `compat-child-setup-complete` 之间的最后阶段。
- `pipe-wait` 的 child PID 与原始信号状态。

最后阶段能缩小范围，但不等于精确 syscall 定位。若本轮仍在辅助程序第一阶段前或回退子进程第一阶段前失败，应将真实 child PID 对应到系统故障记录，继续调查启动入口之前的行为。
挂载隔离错误与本次 SIGSYS 分开复测；目前还没有终端故障修复完成的结论。
