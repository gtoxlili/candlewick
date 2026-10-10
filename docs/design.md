# 配色与视觉语言

Candlewick 的三个窗口、系统栏上的文字和菜单里的涨跌色用的是同一套配色，定义在 `src/index.css` 的 `:root`（浅色）和 `@media (prefers-color-scheme: dark)`（深色）里，页面跟随系统外观。它不再使用 shadcn 的默认色、macOS 的 `-apple-system-*` 语义色或 Windows 的强调色。

## 来源

应用图标：深海军蓝底上一条从紫 → 青 → 薄荷的价格线，末端一个发亮的点。配色就从这条线上取：

| 名字 | 浅色 | 深色 | 用途 |
|---|---|---|---|
| `--glow`（`--primary`） | `#0076af` | `#49c8f3` | 唯一的强调色：控件、折线、当前项、账户钱包条 |
| `--mint`（`--live`） | `#009365` | `#56ecb5` | 「实时」的点、持仓分布条末端亮着的点 |
| `--violet` | `#7456e0` | `#aaa0fb` | 第二序列色 |
| `--amber`（`--busy`） | `#c5770f` | `#f7b83d` | 连接中、读取中、接近强平 |
| `--green` / `--red` | `#008047` / `#d42e3d` | `#30d792` / `#ff6e74` | 涨跌，经 `--up` / `--down` 随「红涨绿跌」互换 |

文字和线是带海军蓝色相的灰（`--ink`，oklch 色相 270），以 alpha 叠在窗口材质上，所以同一组值在任何桌面背景、vibrancy 或 Mica 上都成立：

| 变量 | 含义 |
|---|---|
| `--foreground` / `--muted-foreground` / `--faint-foreground` | 92% / 56% / 36% 的墨色；Tailwind 里对应 `text-foreground`、`text-muted-foreground`、`text-faint` |
| `--border`、`--input` | 10% / 12% |
| `--panel` | 图表和列表所在的「抬起」面板（`panel` 工具类：圆角 2xl + 边线 + 半透明白） |
| `--fill` / `--fill-strong` | 统计块、分段控件和悬停行的填充（`bg-fill`、`pill` 工具类） |
| `--series-1 … 5`、`--series-rest`、`--series-cash` | 持仓分布的序列色：glow、violet、mint、amber、淡紫，其余折成一段，稳定币用最安静的灰 |

数值是在 OKLCH 里选的（浅色 L≈0.52–0.57，深色 L≈0.72–0.85），再换算成十六进制写死，以便 Rust 侧使用同样的值。浅色涨跌色在 vibrancy 的浅灰上对比度约 4.3:1。

## 原生侧

页面之外的颜色由 Rust 画，值必须和 CSS 一致：

- macOS：`platform/macos/ticker.rs` 的 `GREEN` / `RED`，造成动态 `NSColor`（`colorWithName:dynamicProvider:`），用于菜单行里的涨跌数字和菜单栏双排图像；菜单文字本身由系统绘制。
- Windows：`platform/windows/portable/color.rs` 的 `Palette::taskbar`，用于任务栏行情条的涨跌混色、托盘图标的圆点和菜单行首的三角标记。文字色仍是 Windows 11 的 TextFillColor，要和任务栏上其他项一致。
- 行情图：`src/chart/palette.ts` 在运行时读 `--primary`、`--up`、`--down` 交给 Liveline，所以图表自动跟着这里走。

## 语言

三个窗口共用「原生质感的数据极简」：系统字体，数字用 `tabular`；一个大数（34px semibold，变化时闪一下涨跌色）+ 一行说明；分段控件是胶囊（`PillTabs`）；主体内容放在 `panel` 上，列表像盘口一样紧凑、右对齐，数字背后用低透明度的条表示占比；底部一排 `bg-fill` 的统计块。状态用一个点表达：mint 实时、amber 忙、red 出错、faint 过期。

改配色只改 `index.css` 的 `:root` 两段和上面两处 Rust 常量；不要在组件里写颜色字面量。

## 行情图上的层

Liveline 画 K 线；它之上的东西（`src/chart/analysis/`）另画在一块 `pointer-events: none` 的画布上，每帧跟着 Liveline 报告的几何（`onFrame`：缓动后的时间和价格区间）重画，所以拖动、缩放和实时跳动时都贴着 K 线走。颜色全部从 `palette.ts` 读 CSS 变量，画布里不写字面量：

| 东西 | 画法 |
|---|---|
| 十字线 | 1px 墨色 30% 的点线，竖线吸附在 K 线中心；横线跟指针，靠近开高低收 8px 内吸附并画一个小实心点。价格和时间各一个反色标签（墨色填充、背景色文字，圆角 5px，18px 高）贴在坐标轴上 |
| 悬停卡片 | DOM，不是画布：`bg-popover` 加毛玻璃、`rounded-xl`、`text-xs tabular`；首行是时间和这根 K 线相对开盘的涨跌（涨跌色），下面两列开高低收，再一行振幅，开了指标时底部再列各指标在这根 K 线上的值。跟着指针走，离指针 14px，到边缘就翻到另一侧；值由每帧直接写 DOM，不经 React |
| 画线 | 默认 `--violet`（第二序列色，和折线的 glow 区分开），选中换 `--glow` 并在两端画 4.5px 的圆手柄（背景色填充、glow 描边）。水平线在价格轴上有自己的标签；斐波那契从前一端画到右边缘，相邻水平线之间铺由浅到深的色带（alpha 0.047 起每级加 0.012），标签「0.618  价格」放在线上方左端，两端之间一条 35% 的虚线；测量是涨跌色 9% 的矩形加 60% 的边，读数卡放在矩形上方（放不下就下方），第一行涨跌幅和差价用涨跌色，第二行根数和时长用墨色 60% |
| 指标 | 1.25px 线：MA 依次 violet、amber、mint，EMA 墨色；布林带用 glow：外轨 1px 55%，中轨点线，带内填 7%。图左上角一行图例：色点、名字、当前值（悬停时是那根 K 线上的值） |
| 工具栏 | 图表头部行里一枚胶囊：光标、水平线、趋势线、斐波那契、测量，一条分隔线后是指标菜单（Σ），按下的那个像图表类型切换一样抬起成白色。有画线时左边出现橡皮擦，删过东西后出现撤销 |

画布文字用系统字体 11px / 500（标签 10px）；画布拿不到 `tabular`，所以多行数字对齐的地方（悬停卡片、图例）一律做成 DOM。
