# 发布

版本号、tag 和安装包都不用手工处理。main 上每次推送只要改动了应用本身，`.github/workflows/build.yml` 就会算出新版本，构建 macOS 和 Windows 的安装包，发一个 GitHub Release。

## 什么算应用本身

`src`、`src-tauri`、`index.html`、`chart.html`、`package.json`、`pnpm-lock.yaml`、`pnpm-workspace.yaml`、`patches`、`tsconfig.json` 和 `vite.config.ts`。只改文档不会触发构建。只改工作流会完整构建一遍，但不发布。

## 版本号

日常发布只递增修订号 `z`，次版本号 `y` 留给明确的阶段性发布：

- 自动发布：main 上有应用改动时，`feat:`、`fix:` 和其他提交都升 `z`，例如 `0.8.0 → 0.8.1`。一次发布只加一，不按提交数量累加。`!` 和 `BREAKING CHANGE` 也不再自动决定版本跨度。
- 阶段性发布：在 GitHub 的 **Actions → Build Candlewick → Run workflow** 中选择 `main`，将 `bump` 设为 `minor`，例如 `0.8.7 → 0.9.0`。允许在没有新应用提交时主动发布这个里程碑版本。
- 手动运行的默认选项为 `patch`。PR 和其他分支不会发布。

提交信息仍使用 Conventional Commits，方便阅读发布说明，但提交类型不再触发次版本号或主版本号递增。

机器人把新版本写进 `src-tauri/tauri.conf.json`、`package.json`、`src-tauri/Cargo.toml` 和 `src-tauri/Cargo.lock`，提交 `chore: release vX.Y.Z` 并推到 main，安装包从这个提交构建。所以本地推送前先 `git pull --rebase`。

要跳到指定版本（比如 1.0.0），手动把这四个文件改成那个版本再推送。文件里的版本还没有对应的 tag，工作流就原样发布它，不再加一。

发布失败后同样复用文件中的版本号，即使重试时选择了 `minor`，也会先完成尚未打 tag 的版本。完成后再次手动选择 `minor`，才会升到下一个次版本。

## 产物

| 平台 | 安装包 | 签名 |
|---|---|---|
| macOS，Apple 芯片 | `Candlewick_<版本>_aarch64.dmg` | Developer ID 签名，应用和 DMG 都经过公证并装订票据 |
| Windows x64 | `Candlewick_<版本>_x64-setup.exe` | 暂未签名 |
| 自动更新 | `latest.json`、`Candlewick_<版本>_aarch64.app.tar.gz` | 更新包和 Windows 安装包都用更新密钥签名 |

两个平台都在 GitHub 托管的机器上构建，构建前各跑一遍 clippy（有警告即失败）和单元测试，macOS 上还检查 rustfmt。

全部成功后才创建 Release 和 tag。Release 说明里依次是这次的提交、安装方法和两个安装包的 SHA-256。中途失败不会打 tag，下一次推送会原样重发这个版本，版本号不会跳。

## 自动更新

已安装的应用从 `https://github.com/gtoxlili/candlewick/releases/latest/download/latest.json` 读取最新版本。这个文件是每个 Release 的附件，列出各平台更新包的地址和签名。macOS 的更新包是 `.app.tar.gz`，CI 确认应用已经装订了公证票据之后才打包，并解开检查一遍；Windows 直接用安装包。

应用只接受能用公钥验证的更新包，公钥在 `tauri.conf.json` 的 `plugins.updater.pubkey`，私钥是 `release` 环境里的 `TAURI_SIGNING_PRIVATE_KEY`。换掉这对密钥，已经安装的版本就再也收不到更新，所以私钥要另外妥善备份。

应用这边的逻辑在 `src-tauri/src/update.rs`：启动一分半后检查一次，之后每六小时一次；新版本下载并验证后，等到锁屏或显示器关闭、并且没有打开的窗口时才重新启动。不想等的话，下拉菜单和设置页里都能立即重启。设置里关掉「自动更新」后，只在手动点「检查更新」时才会检查。

## Pull request

PR 只构建和测试 Windows 安装包，不签名，也不发布；macOS 版需要签名，只在发布时构建。安装包在运行页面的 Artifacts 里保留 7 天，可以下载到 Windows 上试装。

## 签名凭据

macOS 安装包的签名和公证凭据，以及更新密钥，都放在 GitHub 的 `release` 环境里。这个环境只允许 main 使用，其他分支和 PR 都读不到。

签名还需要 Apple 的 Developer ID G2 中间证书。它是公开证书，构建时从 apple.com 下载，确认能链到系统信任的 Apple 根证书后再装进钥匙串。

## Windows 代码签名

还没有接入。未签名的安装包第一次运行时，SmartScreen 会提示「Windows 已保护你的电脑」，点「更多信息 → 仍要运行」即可，之后的更新不受影响。要接入的话，在 `tauri.windows.conf.json` 的 `bundle.windows.signCommand` 里配置签名命令（比如 Azure Artifact Signing），程序和安装包会一起签名。
