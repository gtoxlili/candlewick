# Coin Tray

在 macOS 菜单栏里看币安现货的实时价格。

- 菜单栏直接显示选中币种的价格，可附带币种名称和 24 小时涨跌幅
- 下拉菜单列出全部币种，点其中一个打开行情窗口：折线或 K 线，时间范围 5 分钟到 1 天，另有 20 档盘口和最近成交
- 设置里搜索添加交易对，最多 30 个，可切换红涨绿跌和登录时启动
- Mac 睡眠、显示器休眠或切换用户时暂停行情连接，恢复后自动重连

启动后只出现在菜单栏，没有 Dock 图标，设置从下拉菜单打开。

行情来自币安公开的现货行情接口，不需要账号或 API key。连接跟随系统代理设置，`binance.com` 连不上时改用
`binance.vision` 的公开行情域名。

## 构建

需要 macOS 15 或更高、Rust 1.98+、Node.js 与 pnpm。

```sh
pnpm install
pnpm tauri dev     # 开发
pnpm tauri build   # 产物在 src-tauri/target/release/bundle/
```

## 代码结构

- `src-tauri/src/`：菜单栏、行情连接、窗口与设置存储
- `src/settings/`：设置窗口
- `src/chart/`：行情窗口，图表基于 [Liveline](https://github.com/benjitaylor/liveline)
- `patches/liveline@0.0.7.patch`：让 Liveline 跟随红涨绿跌配色，并在窗口失焦时降低重绘帧率

## 许可

[GPL-3.0](LICENSE)。本项目与币安无关联。
