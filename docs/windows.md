# Windows 版的实现

Windows 版和 macOS 版是同一个应用，行情、设置、行情窗口的代码完全共用。两边不同的只有「挂在系统栏上的那部分」和窗口外观，全部放在 `src-tauri/src/platform/` 下，macOS 一份，Windows 一份，对外提供同一组函数，调用方不写平台分支。

macOS 的菜单栏在 Windows 上对应任务栏。所以固定的价格直接画进任务栏，放在通知区域左边，和时钟挨着，而不是只藏在托盘图标的提示里。

## 和 macOS 版的对应

| 能力 | macOS | Windows | 代码 |
|---|---|---|---|
| 常驻显示的价格 | 菜单栏状态项的标题或绘制的两排图像 | 嵌进任务栏的行情条 | `platform/windows/ticker.rs`、`draw.rs` |
| 入口图标 | 模板图标 | 托盘图标，按任务栏 DPI 绘制，圆点显示涨跌色 | `notify.rs`、`portable/glyph.rs` |
| 自选下拉菜单 | NSMenu，富文本分栏 | 原生弹出菜单，制表符分栏，左侧涨跌三角 | `menu.rs`、`portable/menu_text.rs` |
| 窗口外观 | 透明窗口加 vibrancy，页面画标题栏，系统红绿灯 | Windows 11 用 Mica，Windows 10 用 Fluent 纯色底；页面画标题栏和 Win11 标题栏按钮 | `platform/windows/window.rs`，页面部分见 [windows-frontend.md](windows-frontend.md) |
| 应用形态 | 无窗口时是 Accessory，没有 Dock 图标 | 无窗口时进入效率模式（EcoQoS） | `platform/windows/mod.rs` |
| 再次启动 | LaunchServices 发 Reopen | 命名互斥量保证单实例，第二个进程通知第一个后退出 | `claim_single_instance` |
| 开机启动 | SMAppService 登录项 | `HKCU\...\Run`，并尊重任务管理器里的开关 | `set_login_item` |
| 系统代理 | SCDynamicStore 的 HTTPS 与 SOCKS | WinHTTP 读到的用户手动代理 | `platform/windows/proxy.rs`、`portable/wininet.rs` |
| 暂停推流 | 睡眠、屏幕休眠、快速切换用户 | 睡眠、显示器关闭、锁屏与切换用户 | `shell.rs` |

## 任务栏上的价格

行情条是任务栏窗口 `Shell_TrayWnd` 的子窗口。做成子窗口，它就跟着任务栏一起显示、隐藏、自动隐藏和随全屏应用消失，不需要自己监听这些状态。它是带逐像素 alpha 的分层窗口，文字直接叠在任务栏的材质上，没有底色块。背景 alpha 是 1/255 而不是 0，因为分层窗口只在 alpha 不为 0 的地方接收点击。

位置取自通知区域窗口 `TrayNotifyWnd` 的左缘，高度占满任务栏。Windows 11 把任务栏设为左对齐时，小组件按钮（天气）会挪到通知区域左边，这时行情条再往左让出 160 DIP。任务栏的布局会在没有通知的情况下变化，比如托盘图标增减，所以 shell 线程每 2 秒核对一次位置和层级。这个定时器允许晚到 0.5 秒，系统可以把它和别的唤醒合并，省电。XAML 做的任务栏内容盖在整条任务栏上，行情条每次核对时确保自己在最上层，只在不是最上层时才调整，避免让 Explorer 反复重绘。

排版照着任务栏时钟来。两排模式下价格和涨跌都是 12 DIP、基线间距 16 DIP，和时钟的两行一致；名称和单行文字是 14 DIP。字体是 Segoe UI Variable Text，Windows 10 上退回 Segoe UI，股票的中文名由 DirectWrite 回退到微软雅黑。数字用等宽字形，价格跳动时宽度不抖。涨跌幅的颜色和 macOS 一样，是在文字颜色里混入 45% 的涨跌色。任务栏矮于 38 DIP（小任务栏按钮）时两排放不下，涨跌幅改为跟在价格后面同一行显示。

鼠标悬停时出现 Windows 11 任务栏按钮那样的圆角底板，按下时变浅，下拉菜单打开期间保持高亮，和点开时钟时一样。左键或右键松开时打开下拉菜单。

绘制用 Direct2D 软件渲染到 32 位位图，灰度抗锯齿，因为背景是半透明的，ClearType 不适用。画面只在内容、配色、DPI、任务栏高度或悬停状态变化时重画。

找不到任务栏窗口时不会猜位置，只保留托盘图标，价格在托盘图标的提示里。竖放的任务栏（Windows 10 可以）也是这样处理。

## 托盘图标与下拉菜单

托盘图标没有用 Tauri 的托盘接口，而是直接调 `Shell_NotifyIconW`。原因有三个方面：Tauri 依赖的 tray-icon 没有启用 `NOTIFYICON_VERSION_4`，键盘用户无法用 Win+B 选中图标后回车打开；它不按 DPI 提供尺寸，由系统缩放后发虚；提示文字满 128 个 UTF-16 单位时缺少结尾的 NUL。自己实现后，图标按任务栏所在显示器的 DPI 取 `SM_CXSMICON`，在 16、20、24、32 像素上分别精确绘制。

图标是应用图标里那条价格线，颜色跟随任务栏的深浅，末端圆点在固定标的有涨跌时显示涨跌色，报价过期时变灰。提示文字第一行是 Candlewick，第二行是固定标的的价格和涨跌，连接出问题时第三行写原因。

下拉菜单是原生 Win32 弹出菜单，每次打开时按当前自选重建；填了交易所 API Key 时，最上面先是一行总资产和一条分隔线，它和自选行一起排版，价格列对齐。每行左边是名称，制表符后面的价格和涨跌由系统右对齐。涨跌幅用数字宽度的 U+2007 补齐到同样的字符数，所以各行价格的右边缘对齐，这和 macOS 用制表位对齐是同一个效果。行首的三角标记用涨跌色表示方向，原生菜单文字本身没法着色。菜单打开期间价格照常刷新：shell 线程在菜单的模态循环里收到渲染消息，就地改掉菜单项文字，再让菜单窗口重绘。连接状态有问题时，自选下面多一行灰色说明，和 macOS 一样。

深色模式下的菜单要调用 uxtheme 只按序号导出的函数，Windows 自家应用和主流 UI 框架都是这么做的，代码只在 1809 及以后的版本上调用它们。

## shell 线程

托盘图标、行情条、下拉菜单，以及对系统广播的监听，都在一个独立线程 `candlewick-shell` 上。原因是行情条是 Explorer 任务栏的子窗口，两个线程的输入队列会被系统连在一起。如果这个线程卡住，任务栏也会跟着卡。因此这条线程上的任何代码都不等待主线程：需要开窗口时，把任务丢给主线程就返回。

这条线程的状态放在 thread-local 的 `RefCell` 里，只做短暂借用。弹出菜单会在内部跑消息循环，窗口过程会被再次调用，所以调用它时不持有借用。线程在调用 Explorer 时也会被系统插入别的消息，这时状态可能正被借用，借不到的消息会重新投递到队列末尾，等当前调用返回后再处理，不会 panic，也不会丢。系统广播一律转成投递消息再处理，原因相同。

它接收的消息有：`TaskbarCreated`（Explorer 重启后重新添加图标、重新挂上行情条）、`WM_SETTINGCHANGE`（深浅色、强调色、任务栏对齐方式变了）、`WM_DISPLAYCHANGE` 与 `WM_DPICHANGED`、`WM_POWERBROADCAST`（睡眠与唤醒，以及 `GUID_SESSION_DISPLAY_STATUS` 报告的本会话显示器开关，远程会话也适用）、`WM_WTSSESSION_CHANGE`（锁屏、解锁、切换用户，锁定和断开分开记录，两者都解除才恢复），以及第二个实例发来的重新打开请求。关闭时 shell 先标记为已关闭，之后到达的渲染或菜单收起都不会再把图标和行情条加回来。

## 窗口

设置、行情和持仓窗口去掉了系统标题栏，但保留了可调整大小的边框和阴影，所以窗口仍有 Windows 11 的圆角，也能从边缘拖动改变大小。页面自己画标题栏，并用 CSS `app-region: drag` 标出可拖动区域。wry 打开了 WebView2 的非客户区支持，这块区域就成了真正的标题栏：拖动时有贴靠布局，双击最大化或还原，右键出现系统菜单。

Windows 11 上窗口背后是 Mica。Windows 10 没有 Mica，而窗口透明与否不取决于效果是否生效，所以 Windows 10 上窗口不透明，页面自己画 Fluent 的纯色底（浅色 `#F3F3F3`，深色 `#202020`）。

另外几处针对 WebView2 的处理：滚动条用 Fluent 覆盖式滚动条；关闭通用自动填充；发布版里关闭浏览器快捷键，F5、Ctrl+R 不会刷新页面，Ctrl+P 不会打印，开发版保留以便调试；页面里非输入框的右键菜单被屏蔽，输入框保留剪切、复制、粘贴。

WebView2 不能在自身的回调里创建新的 WebView，而 IPC 命令正是在这种回调里执行的。所以在 Windows 上，创建窗口一律放到主线程下一轮再做。

窗口最小化时，app 把 WebView2 设为不可见。WebView2 不会自己这样做，不设的话最小化的行情窗口会继续绘制动画；设了之后页面进入 hidden 状态，和 macOS 上一样在一分钟后暂停推流。同时把 WebView2 的内存目标调到 Low，让它释放缓存，还原时再调回 Normal。

最后一个窗口关掉后进程回到效率模式，3 秒后压缩堆并修剪工作集，WebView2 自己的进程会自行退出。

## 系统服务

代理读取的是用户在「设置 → 网络和 Internet → 代理」里的手动代理，也就是 WinINet 的配置，通过 `WinHttpGetIEProxyConfigForCurrentUser` 获得，每次连接时重新读取。服务器字符串的解释和浏览器一致：优先 `https=` 项，其次不带协议名的那一项（Clash Verge、v2rayN 写的就是这种），最后才用 `socks=` 项。WinINet 把 `socks=` 当作 SOCKS4，这里按 SOCKS5 连接，常见的本地代理两种都支持。绕过列表支持 `*` 通配和 `<local>`。自动配置脚本（PAC）和 macOS 版一样不处理。

开机启动写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 下名为 `Candlewick` 的值。任务管理器和「设置 → 应用 → 启动」有自己的开关，存在 `StartupApproved\Run` 里，首字节为偶数表示开启。读取状态时两处都看；打开时两处都写，免得任务管理器里残留的关闭状态继续生效。值名必须和产品名一致，因为 Tauri 的卸载程序按产品名删除 Run 值，升级时不删。任务管理器那一项由 `windows/installer-hooks.nsh` 在卸载时删掉。首次从安装位置运行时自动注册一次开机启动，判断依据是程序旁边有卸载程序 `uninstall.exe`，这和 macOS 上只在「应用程序」目录里注册是同一个考虑。

凭证和 macOS 一样是设置旁边的明文 `credentials.json`。它在 `%APPDATA%` 下，这个目录本来就只有当前用户能读，所以不再另做加密。

Windows 11 会把新装应用的托盘图标先收进溢出区，所以安装后第一次从安装位置运行时会自动打开设置窗口，让用户知道应用已经在运行。它和注册开机启动是同一时机，共用同一个标记文件，之后的启动都不再打开，包括开机自启。开发时直接运行不算安装，不会弹出。

## 构建与发布

需要 Rust 1.98 以上的 MSVC 工具链、Visual Studio 的 C++ 生成工具、Node.js 与 pnpm。Windows 11 自带 WebView2，Windows 10 通常也已经有。

```powershell
pnpm install
pnpm tauri dev
pnpm tauri build   # 安装程序
```

安装程序在 `src-tauri/target/release/bundle/nsis/`，文件名形如 `Candlewick_0.5.0_x64-setup.exe`。Windows 专用的打包配置在 `src-tauri/tauri.windows.conf.json`，构建时与 `tauri.conf.json` 合并：只打 NSIS 安装包；按当前用户安装，不需要管理员权限；安装界面为简体中文；WebView2 低于 125 版时由安装程序更新，Fluent 覆盖式滚动条和标题栏的非客户区支持都依赖这个版本。

程序清单在 `src-tauri/windows/app.manifest`，由 `build.rs` 嵌入。它声明了 Windows 10/11 兼容性、Per-Monitor V2 DPI 和 Common Controls 6。兼容性声明必不可少，没有它系统会拒绝创建分层子窗口，行情条就挂不上任务栏。清单还启用了 Segment Heap，Windows 10 2004 起生效，对整天运行的进程来说占用和碎片更少。`tauri build` 静态链接 VC 运行库，干净的系统上不需要另装 VC++ 运行库。

`src-tauri/icons/icon.ico` 含 16、20、24、32、40、48、64、256 八个尺寸，256 用 PNG 压缩。它和 macOS 图标用的是同一套插画，但去掉了 macOS 风格的投影，底板占画布约 92%，更接近 Windows 图标的比例；32 像素及以下用 `app-icon-small.svg` 的简化插画。生成方法是用 Chrome 无头模式把 SVG 在每个目标尺寸上直接渲染成 PNG，再合成 ICO。

自动更新时，应用先把托盘图标和任务栏上的价格移除，再以 passive 模式运行验证过的安装包：只显示一个进度条，装完自动重新打开，全程不需要点任何按钮。

安装包暂未签名，第一次运行时 SmartScreen 会拦一下，需要点「更多信息 → 仍要运行」。接入代码签名的做法见 [release.md](release.md)。

CI（`.github/workflows/build.yml`）在 GitHub 的 Windows 机器上构建 x64 安装包，先跑 clippy 和单元测试。每个 PR 都会构建，安装包在运行页面的 Artifacts 里，可以下载到 Windows 上试；合并到 main 后，它随新版本一起发布。版本号和发布流程见 [release.md](release.md)。

## 已知限制

- 自绘的最大化按钮悬停时不会弹出 Windows 11 的贴靠布局面板。WebView2 让网页声明这类按钮的接口还在预览阶段，Tauri 也没有接入。拖到屏幕顶端和 Win+Z 仍然可用。
- 行情条只出现在主显示器的任务栏上，托盘图标也是如此。
- 任务栏不会给第三方窗口让位。Windows 11 上任务栏左对齐且打开的应用很多时，任务按钮可能延伸到行情条下面；Windows 10 的任务按钮区一直延伸到通知区域，行情条总是盖住它最右端的一段，只有按钮排到那里时才看得出来。
- 竖放的任务栏上不显示行情条。
- 不支持代理自动配置脚本（PAC）。
