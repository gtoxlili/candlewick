# Windows 版的页面

设置、行情和持仓窗口的页面在两个平台上是同一份代码。平台差异集中在几处：`src/lib/platform.ts`、`src/index.css` 里的 Windows 段、`TitleBar` 与 `CaptionButtons`、`Switch`，以及 `Picker`。Rust 侧的实现见 [windows.md](windows.md)。

## 平台常量

页面按平台分别构建。`vite.config.ts` 根据 Tauri CLI 传来的 `TAURI_ENV_PLATFORM` 注入构建时常量 `__WINDOWS__`，类型声明在 `src/globals.d.ts`；直接运行 `vite` 时按本机系统取值。这个常量在每个模块里都会被替换成字面量 `true` 或 `false`，另一个平台的分支在打包前就会被删掉，只在那个分支里用到的模块也不会进包。比如 macOS 的包里既没有 `CaptionButtons`，也没有 Tauri 的窗口接口。

不要把它包成导出常量再用，比如 `export const IS_WINDOWS = __WINDOWS__`。这样的常量到打包后期才会折叠，分支虽然删了，分支里引入的模块却已经进了包。标题栏图标用 `?inline` 引入也是这个原因：图片文件在模块加载时就会输出，内联之后才能随分支一起去掉。

构建目标也随平台切换：macOS 是 `safari26`，Windows 是 `chrome125`，和安装程序要求的 WebView2 最低版本一致。Tailwind 按源码里出现的类名生成样式，不区分平台，所以每个平台的 CSS 里都带着对方的规则，只是不会命中。

## 运行时信息

窗口背后的材质和系统强调色，要到运行时才知道。Rust 在页面加载前注入 `window.__CANDLEWICK__`：

```ts
{ backdrop: "mica" | "solid", accent: { onLight: "#005fb8", onDark: "#60cdff" } }
```

`backdrop` 表示窗口背后有没有 Mica，只有 Windows 11 有。`accent` 是系统强调色在浅色和深色表面上的取值。WebView2 里的 CSS `AccentColor` 拿不到系统强调色，所以由 Rust 读取。用户改了强调色，应用会发出 `system-colors` 事件，载荷结构和 `accent` 相同。

两个入口都在 `createRoot` 之前调用 `setupPlatform()`，它负责：

- 给根元素加 `data-platform`
- Windows 上加 `data-backdrop`，写入 `--accent-on-light` 与 `--accent-on-dark`；强调色变化时在 `window` 上派发 `candlewick:system-colors`（常量 `SYSTEM_COLORS_EVENT`），行情窗口据此重新取色
- Windows 上维护 `data-active`，窗口失焦时为 `false`
- Windows 上屏蔽输入框以外的右键菜单，WebView2 自带的「返回、刷新、打印、检查」不该出现在应用窗口里

## 根元素属性

| 属性 | 取值 | 用途 |
|---|---|---|
| `data-platform` | `macos`、`windows` | 选择平台样式 |
| `data-backdrop` | `mica`、`solid` | `solid` 时页面自己画窗口底色 |
| `data-active` | `true`、`false` | 失焦时标题栏变淡 |

## 设计变量

字体栈两个平台共用：`-apple-system, BlinkMacSystemFont, "Segoe UI Variable Text", "Segoe UI", "PingFang SC", "Microsoft YaHei UI", …`。macOS 上命中系统字体，中文回退到苹方。Windows 上命中 Segoe UI Variable（Windows 10 上是 Segoe UI），中文回退到微软雅黑。

小号文字用 `text-2xs`，macOS 上是 11px，Windows 上是 12px，后者是 Windows 字号阶梯里最小的一级。Windows 上 `text-xs`、`text-sm`、`text-base` 分别是 12、14、14px，对应 Fluent 的 Caption 与 Body。

Windows 的颜色取自 Windows 11 的设计 token，写在 `:root[data-platform="windows"]` 里：

| 变量 | 浅色 | 深色 | 对应 token |
|---|---|---|---|
| `--foreground` | `rgb(0 0 0 / .894)` | `#fff` | TextFillColorPrimary |
| `--muted-foreground` | `rgb(0 0 0 / .62)` | `rgb(255 255 255 / .786)` | TextFillColorSecondary |
| `--card` | `rgb(255 255 255 / .7)` | `rgb(255 255 255 / .0512)` | CardBackgroundFillColorDefault |
| `--border` | `rgb(0 0 0 / .0578)` | `rgb(255 255 255 / .0837)` | DividerStrokeColorDefault |
| `--primary` | `--accent-on-light` | `--accent-on-dark` | AccentFillColorDefault |
| `--primary-foreground` | `#fff` | `#000` | TextOnAccentFillColorPrimary |
| `--window` | `#f3f3f3` | `#202020` | SolidBackgroundFillColorBase |
| `--subtle-fill-secondary` | `rgb(0 0 0 / .0373)` | `rgb(255 255 255 / .0605)` | SubtleFillColorSecondary |
| `--subtle-fill-tertiary` | `rgb(0 0 0 / .0241)` | `rgb(255 255 255 / .0419)` | SubtleFillColorTertiary |
| `--flyout-stroke` | `rgb(0 0 0 / .0578)` | `rgb(0 0 0 / .2)` | SurfaceStrokeColorFlyout |
| `--flyout-shadow` | 2px 与 16px 两层 | 同左，更深 | Flyout 的阴影 |

需要强调色时用 `var(--primary)`，不要直接写 `AccentColor`，它只在 WebKit 里代表系统强调色。涨跌色 `--up`、`--down` 两个平台通用，和 Windows 任务栏上的价格用的是同一组颜色。

## 组件

`TitleBar` 有两种用法：

```tsx
<TitleBar title="设置" divider={scrolled} />               // 普通标题
<TitleBar className={cn(TITLE_INSET, "gap-2")} maximizable> // 自定义内容
  …
</TitleBar>
```

- `title`：macOS 上居中加粗；Windows 上左侧是 16px 的应用图标（`src/assets/window-icon-*.png`，按缩放取 1x、2x、3x），后面跟 12px 的标题
- `maximizable`：只影响 Windows，决定有没有最大化按钮
- `TITLE_INSET`：标题栏内容的起始位置，macOS 上要让出红绿灯（`pl-[80px]`），Windows 上是 `pl-4`
- Windows 上，标题栏末尾自动带上 `CaptionButtons`

Windows 上的标题栏靠 CSS `app-region: drag` 成为系统标题栏，拖动、贴靠、双击最大化和右键系统菜单都交给系统处理。代价是标题栏里的元素默认收不到鼠标事件。`index.css` 给 `button`、`a`、`input`、`select`、`textarea`、`label` 以及 `role="button"`、`role="tab"` 设了 `no-drag`。往标题栏里放别的可交互元素时，要么用这些标签，要么把选择器补进那条规则。`data-tauri-drag-region` 是给 macOS 拖动用的，在 Windows 上是后备。

`CaptionButtons` 只在 Windows 上渲染，照 Windows 11 的标题栏按钮做：46×32px，字形是 Segoe Fluent Icons（Windows 10 上退回 Segoe MDL2 Assets）的 E921、E922、E923、E8BB，悬停和按下用上表的两个 subtle fill，关闭按钮悬停时是 `#C42B1C` 底配白色字形。按钮不进 Tab 顺序，和系统按钮一样用 Alt+空格和 Alt+F4 操作。最大化状态通过 `onResized` 同步，最大化后显示「还原」。窗口失焦时，标题栏内容和未悬停的按钮一起降到 45% 不透明度。

`Picker` 是从一段页面内容上弹出的单选菜单，行情窗口的标的切换和盘口的合并档位都用它。macOS 上是在内容上盖一个透明的 `<select>`，弹出系统菜单；Windows 上用 shadcn 的 Select（`src/components/ui/select.tsx`，由 shadcn CLI 生成，不改动），触发器保留页面自己的样式，箭头换成 Windows 组合框的单向下箭头。弹出的列表在 `index.css` 里按 Windows 11 组合框的下拉层来画：8px 圆角的亚克力浮层，32px 的选项，悬停和按下用 subtle fill，当前项左侧是强调色竖条，打开时当前项对齐到触发器上。它盖在标题栏上时，CSS 把它排除在可拖动区域之外。

`Switch` 按平台选用两套类名。Windows 那套照 Windows 11 的开关：40×20px 的描边轨道，关闭时是 12px 的实心圆点；打开时轨道填强调色，圆点变成 `--primary-foreground`。悬停时圆点放大到约 14px，按下时横向拉长。`size="sm"` 是 32×16px，圆点 10px。

## 文案

「菜单栏」用常量 `BAR`，Windows 上显示「任务栏」。设置页里另有三处按平台区分：自选的说明、「红涨绿跌」的说明，以及开机启动开关的名称和说明。

## 权限

`src-tauri/capabilities/caption-buttons.json` 用 `platforms: ["windows"]` 限定只在 Windows 上生效，给设置、行情和持仓窗口开放 `minimize`、`toggle-maximize`、`is-maximized`、`close`。

## 和 macOS 表现不同的地方

- 触控板捏合在 WebView2 里是带 `ctrlKey` 的 `wheel` 事件，`PriceChart` 已经处理；WebKit 的 `gesture*` 事件在 Windows 上不会触发
- 滚动条用的是 Rust 侧开启的 WebView2 Fluent 覆盖式滚动条，页面不用写滚动条样式
- 窗口最小化时，应用会把 WebView2 设为不可见，页面的 `visibilityState` 随之变为 `hidden`，暂停推流、停止动画都和 macOS 一致

## 已知限制

- 最大化按钮悬停时不会弹出 Windows 11 的贴靠布局面板，WebView2 的相关接口还在预览阶段
- 输入框和焦点框沿用 macOS 风格的样式
