# Windows 上的窗口

设置、行情和持仓三个窗口和 macOS 上是同一份 GPUI 视图（`src/ui/`），窗口参数在 `src/window.rs`：设置 460×640、不能缩放，行情和持仓 980×640（最小 760×480 和 800×500），都居中打开，每种窗口只有一个，再次打开是把已有的提到前面。

- 标题栏：窗口自己画（GPUI Kit 的 `TitleBar`，34px 高），左边是标题，右边是最小化、最大化 / 还原和关闭。这块区域交给系统当标题栏：拖动、贴靠、双击最大化、右键系统菜单都由 Windows 处理；三个按钮用系统的非客户区命中测试，和资源管理器窗口上的按钮行为一致，Windows 11 上最大化按钮悬停会弹出贴靠布局。
- 材质：窗口背板是 Mica（`WindowBackgroundAppearance::MicaBackdrop`）。GPUI 只在 Windows 11 22H2 及以后设置它。
- 字体：Segoe UI Variable Text，系统没有这个字体时用 Segoe UI；正文 14px（macOS 上是 13px）。
- 最后一个窗口关掉后应用只剩任务栏：托盘、任务栏行情条和菜单照常，进程进入效率模式（EcoQoS）；再打开窗口时退出。

配色两个平台共用一套，见 [design.md](design.md)。
