# 行情数据源：接入说明与调研记录

币安和长桥调研于 2026-09-26，Bybit 和 OKX 于 2026-10-01。结论会随时间变化，接入前请在目标网络（大陆直连或代理）下实测一次。

## 怎么接入一个新数据源

所有取数都在 Rust 端，WebView 只通过 IPC 拿数据。

加密货币交易所同一时间只用一家：设置里的 `Settings.exchange` 选定，自选只保留这家的币对（`Settings::validated` 会移除别家的，换交易所就是这样清掉旧币对的），搜索也只搜这家和长桥。

### 加一家加密货币交易所

三家交易所的公开行情形状相同，共用的部分在 `src-tauri/src/market/crypto/`：交易对搜索（`pairs.rs`）、菜单栏报价的 WS 驱动（`quotes.rs`）、行情窗口实时流的驱动（`live.rs`）和本地盘口（`book.rs`）。新交易所只写它不同的地方：

1. `ProviderId` 加一个变体，并放进 `ProviderId::EXCHANGES`；`provider()` 返回实现。
2. 实现 `crypto::Exchange`（`Provider` 随之而来）：
   - 常量：REST 主机和 WS 服务器（按顺序，失败后轮换）、保活帧 `PING`（交易所自己发 ping 的填 `None`）、K 线周期（与时钟对齐、日线从 UTC 0 点开始）。
   - `symbol`：交易所写交易对的方式（`BTCUSDT`、`BTC-USDT`），`validate` 和手动输入都靠它。
   - `pairs` / `history` / `recent_trades`：REST。
   - `quotes_path` / `quotes_subscribe` / `quote`：菜单栏报价连哪、连上后发什么订阅、怎么从一帧里读出最新价和 24 小时前的价格。
   - `live`：行情窗口的 `Session`。它是不碰 I/O 的状态机：`connected` 时清空并订阅，`frame` 处理每一帧，通过 `Out` 发帧、发事件、交成交、或者起一个后台请求（结果回到 `fetched`）。连接、重连退避、保活、成交攒批都由 `live::run` 负责，所以会话逻辑可以直接用帧文本做单测。
3. 前端 `src/lib/api.ts` 的 `Exchange` 和 `EXCHANGES` 加上它，设置页的选择器就有了。

### 加别的数据源

实现 `Provider` trait：

- `validate`：校验设置页传来的自选项。
- `search`：返回候选；`notes` 说明搜不全的原因。
- `drop_search_cache`：设置窗口关闭时释放搜索缓存。
- `watch`：菜单栏（Windows 上是任务栏）报价的常驻任务，读取 `FeedControl` 里的 symbols、paused 和 credentials，写 `model.quotes`，用 `market::set_status` 报告状态。
- `chart_spec` / `history` / `recent_trades` / `stream`：行情窗口的能力说明、K 线翻页、最近成交和实时流。

前端 `src/lib/api.ts` 的 `ProviderId`、`instrumentLabel`、`marketLabel` 加上对应分支。

### 几个容易踩错的约定

- `IntervalSpec.aligned`：K 线是否与时钟对齐（`time % secs == 0`），前端据此自己分桶；不对齐的周期由前端每 30 秒或成交越界时向数据源补拉最新几根。`regular_only`：盘前盘后成交不计入该周期。
- `history`：数据源每页上限比请求的 `limit` 小时就返回能给的（OKX 一页 300 根）；只有返回空才表示没有更早的了，前端据此停止往前翻。
- `ChartSpec.day_offset`：日线开盘时刻相对 UTC 的秒数，用于日期标签。
- `ChartSpec.book_steps`：盘口可选的合并档位，从细到粗；`LiveEvent::Book.books` 按同样顺序各给一份合并好的盘口。留空表示不合并，`books` 只放一份原样的盘口。
- 本地盘口的价格存成 10^-18 为单位的 `u128` 整数（Bybit 的 tick 细到 10^-13，币安 BTC/IDR 价格到 15 亿），同一价格总能落到同一档；转回 `f64` 时与直接解析原字符串的结果一致。
- `Trade.id` 必须随时间递增、对同一笔成交稳定（查询和推送会各来一次，前端按 id 去重），而且小于 2^53，否则 WebView 里的数字会把相邻的 id 并成一个。
- 需要账户的数据源：凭证放在 `credentials.rs` 管理的 `credentials.json`（0600），缺凭证时用 `Status::Unavailable` 提示，不要重试空转。
- `Quote.open` 是涨跌幅的参考价：币是 24 小时前的价格，股票是最近一次收盘价；`Quote.session` 标注盘前、盘后、夜盘。
- 菜单栏每秒最多重绘一次：OKX 每个币对每秒最多推 10 次，币安是每秒一批。

## 已接入

三家交易所都是公开行情，不需要账号。菜单栏一条 WS 订阅所有自选币对的 ticker；行情窗口一条 WS 订阅成交、盘口和 ticker。涨跌幅都相对 24 小时前的价格，日线都从 UTC 0 点开始。

### 币安

- REST：`api.binance.com`，备用 `data-api.binance.vision`；WS：`stream.binance.com`，备用 `data-stream.binance.vision`。服务器每 20 秒发一次 ping，tungstenite 自动回 pong。
- 订阅写在 URL 里（`/stream?streams=…`）。菜单栏用 `<symbol>@miniTicker`；行情窗口用 `@aggTrade`、`@depth`（增量，每秒一次）、`@ticker`，连上后用 REST 补一次 24hr 统计。
- 盘口按币安的 "How to manage a local order book correctly" 在本地维护：`/api/v3/depth?limit=5000`（权重 250）做快照，增量按 `U`/`u` 衔接，断档后退避重拉。快照之外的价位只有变动时才知道，所以合并时每边只算到快照覆盖的范围，价格走到快照边缘也重拉。合并档位是 tick 的 1、10、100、1000 倍（三家相同）。
- K 线一页 1000 根，周期 1 秒到 1 天；交易对列表来自 `exchangeInfo`（数 MB），只在搜索时下载，设置窗口关闭即释放（三家相同）。

### Bybit（V5 API，`src-tauri/src/market/bybit.rs`）

旧版接口已下线，只用 V5（`category=spot`）。

- REST：`api.bybit.com`，备用 `api.bytick.com`；WS：`wss://stream.bybit.com/v5/public/spot`，备用 `stream.bytick.com`。响应包装 `{retCode, retMsg, result}`，`retCode != 0` 为错误，此时 `result` 是 `{}`。
- 订阅：`{"op":"subscribe","args":["tickers.BTCUSDT"]}`。现货每个请求最多 10 个参数，而且**一个参数无效整个请求都失败**，所以菜单栏每个币对单独发一个请求。要求每 20 秒发 `{"op":"ping"}`，否则约 10 分钟后断开。
- 菜单栏和统计都用 `tickers.{symbol}`：`lastPrice`、`prevPrice24h`（24 小时前）、`volume24h`、`turnover24h`。`price24hPcnt` 是小数且只到 4 位，涨跌幅自己算。
- 成交：`publicTrade.{symbol}`，`S` 是吃单方向。成交 id（REST 的 `execId`、推送的 `i`）形如 `2290000001221173210`：前 3 位是撮合分片，后 16 位是分片内所有币对共用的计数器，单个币对内递增但不连续。整数超过 2^53，所以只取后 16 位。
- 盘口：`orderbook.1000.{symbol}`，200 毫秒一次（现货另有 1、50、200 档）。先推 `snapshot` 再推 `delta`，`u` 每条加 1；数量为 0 删档；服务端始终维持前 1000 档（档位被挤出时会发删除），所以满 1000 档的一侧在最差一档之后视为未知。`u` 不连续就退订再订阅，换一份新快照；收到新的 snapshot（包括服务重启时 `u = 1` 的那份）直接覆盖。
- K 线：`/v5/market/kline`，周期 `1`、`5`、`15`、`60`、`240`、`D`，没有 1 秒；一页 1000 根，倒序返回；`end` 包含在该时刻开盘的那根，所以传 `end - 1`。最近成交 `/v5/market/recent-trade` 现货最多 60 条。
- 交易对：`/v5/market/instruments-info?category=spot` 一次返回全部（约 530 个，现货不分页），`status == "Trading"`，tick 在 `priceFilter.tickSize`，最细到 10^-13（BABYDOGEUSDT）。

### OKX（`src-tauri/src/market/okx.rs`）

- REST：`openapi.okx.com`（文档现在给的地址），备用 `www.okx.com`；美国和澳洲用户的 `us.okx.com`、欧洲的 `eea.okx.com` 没有接入。响应包装 `{code, msg, data}`，`code != "0"` 为错误。
- WS：`wss://ws.okx.com:8443/ws/v5/public`；同一主机的 443 端口实测也通，作为备用，给只放行 443 的代理用（文档没写）。30 秒没有数据就断开，所以每 20 秒发一个字符串 `ping`，回 `pong`。
- 订阅：`{"op":"subscribe","args":[{"channel":"tickers","instId":"BTC-USDT"}]}`，每个参数各自成败，无效的只回一个 `event: error`（`60018`），所以菜单栏一个请求就够。交易对写作 `BTC-USDT`。
- 菜单栏和统计用 `tickers`：`last`、`open24h`（24 小时前）、`vol24h`（基础币）、`volCcy24h`（现货是计价币）；最快 100 毫秒一次。
- 成交用 `trades`：同一吃单、同一价格的多笔合并成一条，`tradeId` 是其中最后一笔的；REST `/api/v5/market/trades`（最多 500 条）是逐笔的。两边 id 都在 2^53 以内。`trades-all` 逐笔但在 business 端点，要另开一条连接，没用。
- 盘口用 `books`：400 档，先推 `snapshot` 再 100 毫秒一次 `update`，服务端维持前 400 档。`checksum` 已废弃（固定为 0），衔接只看序号：`prevSeqId` 等于上一条的 `seqId` 才能接上；约 60 秒没变化会推一条空的心跳（`prevSeqId == seqId`）；维护时序号可能重置（`seqId < prevSeqId`），照样按 `prevSeqId` 衔接。接不上就退订再订阅。档位是 `[价格, 数量, "0", 订单数]`。更深或逐笔的 `books-l2-tbt` 等需要登录。
- K 线：最新一页用 `/api/v5/market/candles`，往前翻用 `/api/v5/market/history-candles` 加 `after`（早于该时刻），两者一页都最多 300 根、倒序返回、都支持 1 秒（历史 1 秒只到 3 个月前）。日线必须用 `1Dutc`：`1D` 按香港时间 0 点开盘。限频 40 次 / 2 秒和 20 次 / 2 秒。
- 交易对：`/api/v5/public/instruments?instType=SPOT`（约 1150 个），`state == "live"`，tick 在 `tickSz`，最细到 10^-12。

### 长桥 OpenAPI（自写客户端，`src-tauri/src/market/longbridge/`）

没用官方 SDK：实测它会让安装包增大 2.3 MB，常驻内存多 5 MB，还会另开一个线程数等于核数的全局 runtime（多 10 个线程）。协议细节以 SDK 源码为准（`longbridge-wscli`、`longbridge-httpcli`、`longbridge-proto`、`longbridge-candlesticks`，MIT/Apache）。

接入点与签名：

- HTTP `https://openapi.longbridge.com`，大陆 `https://openapi.longbridge.cn`；行情 WS `wss://openapi-quote.longbridge.com/v2`（大陆 `.cn`），URL 带 `?version=1&codec=1&platform=9`。判断大陆：`GET https://geotest.lbkrs.com` 返回成功即用 `.cn`。
- 签名头：`X-Api-Key`、`Authorization`（access token）、`X-Timestamp`（秒）、`X-Api-Signature`。待签串 `方法|路径|查询串|authorization:{token}\nx-api-key:{key}\nx-timestamp:{ts}\n|authorization;x-api-key;x-timestamp|`（有请求体再接体的 SHA-1 十六进制）；先 SHA-1，得到 `HMAC-SHA256|{sha1hex}`，再用 App Secret 做 HMAC-SHA256，头的值为 `HMAC-SHA256 SignedHeaders=authorization;x-api-key;x-timestamp, Signature={hex}`。另带 `x-dc-region`：任一凭证以 `us_` 开头为 `us`，否则 `ap`。
- 响应包装 `{code, message, data}`，`code != 0` 为错误。
- OTP：`GET /v1/socket/token` → `{otp, limit, online}`，`online >= limit` 表示连接数已满。Token 刷新：`GET /v1/token/refresh?expired_at=<RFC3339>` → `{token}`。Token 形如 `hk_m_<JWT>`，到期时间在 JWT 的 `exp`，默认 90 天。

WS 帧（大端）：首字节低 4 位为类型（1 请求、2 响应、3 推送），`0x10` 表示体后跟 24 字节签名，`0x20` 表示体经过 gzip；随后是命令码（u8），然后：

- 请求：request id（u32）、超时毫秒（u16）、体长（u24）、体。
- 响应：request id（u32）、状态（u8，0 为成功，否则体是 `Error{code, msg}`）、体长（u24）、体。
- 推送：体长（u24）、体。

会话：

- 先 AUTH（命令 2）`{token: otp, metadata: {accept-language, need_over_night_quote: "true"}}` → `{session_id, expires(ms)}`，再 QUOTE_PROFILE（命令 4）拿各市场行情包和服务端下发的限速表。
- 服务端发 WebSocket Ping，超过 120 秒没收到视为断开。
- SDK 断线后固定 2 秒重连，会话未过期时用 RECONNECT（命令 3）复用；我们的实现是指数退避，每次取新 OTP 重新 AUTH，并补发订阅。
- 命令码：6 订阅、7 退订、10 静态信息（中文名）、11 报价、14 盘口、17 成交、19 K 线、27 历史 K 线（按偏移或日期），101/102/104 为报价、盘口、成交推送。订阅类型：1 报价、2 盘口、4 成交。

推送语义与实测踩坑：

- 报价推送是字段补丁：值为 0 或空的字段表示没变；`tag == 1` 是收盘修正（EOD），不作为实时变化。
- 盘口推送按 `position` 增量更新，价格为空表示该档清空；**查询盘口（命令 14）的响应不带 position**，要按顺序编号，否则所有档位会互相覆盖成一档。盘口推送要等快照到了再合并。
- 成交推送是追加；成交没有 id，我们用 `时间秒 << 20 | 内容哈希` 生成，这样同一笔成交无论从查询还是推送来，id 都一致。
- 服务端不推 K 线，SDK 在本地用成交合成：分钟周期以各交易时段的开始时间为起点分桶，日、周、月按交易所当地零点；盘前、盘后、夜盘只计入分钟周期。
- 历史 K 线按偏移查询时，`date`（YYYYMMDD）和 `minute`（HHMM）都是交易所当地时间（美东要算夏令时），`direction` 为 0 表示往前。请求里的 `trade_session`：0 只要盘中，100 包括全部时段。
- 美股快照里的 `pre_market_quote`、`post_market_quote`、`over_night_quote` 各自带 `prev_close`；取时间最新的一个显示，涨跌幅相对它的 `prev_close`（即最近一次常规收盘）。

账户与权限：

- 需要长桥综合账户。2025-09-24 起，只持大陆身份证、没有海外身份的新用户基本无法开户，存量账户不受影响。模拟盘账户的 Token 也能拿行情。
- 实测一个大陆账户的权限：美股 LV1（纳斯达克实时成交和最优一档，含夜盘，仅限 OpenAPI）、港股 LV2（十档，限大陆）、A 股 LV1（五档，限大陆）。默认的港股基础行情（BMP）延迟 15 分钟、没有推送。
- 条款：行情仅限个人非商业用途、不得转发。应用只作为用户自带凭证的客户端。

## 候选数据源（未接入）

结论先说：没有一个股票数据源能同时做到免费、实时、条款允许、大陆可直连。非官方接口都在网页抓取的灰色地带，放进公开仓库有条款风险，而且随时可能被封。

### A 股（非官方，均无需 key，只能 REST 轮询）

| 来源 | 接口 | 数据 | 限制与风险 |
|---|---|---|---|
| 腾讯 | 快照 `http://qt.gtimg.cn/q=sh600519`（含五档）；日/周/月 K `https://web.ifzq.gtimg.cn/appstock/app/fqkline/get?param=sh600519,day,,,320,qfq`（支持区间）；分钟 K `ifzq.gtimg.cn/appstock/app/kline/mkline?param=sh600519,m5,,320` | 准实时，五档，分钟 K 最多 320 根；逐笔可参考 AkShare 用的 `stock.gtimg.cn/data/index.php` | 目前社区公认最稳；当日分时 `minute/query` 自 2026-09 起被 WAF 拦截（501）；没有找到公开的搜索接口；只有通用版权条款 |
| 东方财富 | 报价 `push2.eastmoney.com/api/qt/stock/get?secid=1.600519`，批量 `api/qt/ulist.np/get?secids=…`；K 线 `push2his.eastmoney.com/api/qt/stock/kline/get?secid=…&klt=101&fqt=1&beg=…&end=…&lmt=…` | secid 覆盖 A 股（`0.` 深、`1.` 沪）、美股（`105.` 纳斯达克、`106.` 纽交所、`107.` 美股 ETF）、港股（`116.`）；K 线可按区间翻页，最灵活；五档 | 反爬最凶（2026-09-23 前后失败率 56–81%，`push2delay` 是延时备用通道）；美股约晚 15 分钟；法律声明明写未经交易所书面同意不得传播行情 |
| 新浪 | `hq.sinajs.cn/list=sh600519`（需 `Referer: https://finance.sina.com.cn`）；K 线 `money.finance.sina.com.cn/quotes_service/api/json_v2.php/CN_MarketData.getKLineData?symbol=sh600519&scale=5&ma=no&datalen=1023`；搜索 `suggest3.sinajs.cn` | 五档，K 线只能取最近 1023 根 | 云服务器 IP 常见 403，2026-09 仍有 403 报告；条款禁止未授权转载和商用 |
| 雪球 | `stock.xueqiu.com/v5/stock/quote.json`、`…/chart/kline.json`（需 `xq_a_token` Cookie，可匿名访问首页取得） | K 线支持区间；搜索覆盖 A 股、港股、美股 | 阿里云 WAF 滑块验证；服务协议明文禁止爬虫 |

### 美股（官方 API，均需用户自己注册 key；大陆能否直连都未实测）

| 来源 | 免费档 | 时效 | 备注 |
|---|---|---|---|
| Alpaca | 需模拟盘账户（无需完整 KYC）；REST 200 次/分，WS 30 个标的 | 实时，但只有 IEX 一家交易所的成交 | K 线可回溯到 2016；只有 L1 盘口；没有关键词搜索（要拉全量 assets 过滤）；大陆能否注册未确认。**若要免费的官方实时美股，最现实的是它** |
| Finnhub | 60 次/分，WS 50 个标的（官方口径有冲突） | 报价实时，无 bid/ask | 自 2025-04 起免费版 K 线返回 403；条款禁止任何业务用途 |
| Tiingo | 50 次/时，1000 次/天，每月 500 个标的 | 2025-02 起免费档为衍生参考价（`tngoLast`） | 日线 30 年以上，分钟 K 约 2000 个点；条款要求数据只存内存、不得持久化 |
| Twelve Data | 8 次/分，800 次/天；WS 仅试用 | 实时，但只覆盖约 5% 的全美成交 | 搜索覆盖 A 股代码；条款明确免费层禁止商用 |
| Polygon（2025-10 更名 Massive） | 5 次/分 | 免费档只有收盘数据，实时要 199 美元/月 | 条款禁止用来为他人构建应用 |
| Alpha Vantage | 25 次/天 | 收盘级，分钟数据是付费端点 | 有 `SYMBOL_SEARCH`；个人非商业授权 |
| FMP | 250 次/天 | 以收盘数据为主 | 多用户的公开应用需要商业审核 |
| Yahoo（非官方） | `query1/query2`，另有 streamer WS（protobuf） | 接近实时 | 需要 crumb 和 cookie，2024 年底 v7 批量报价开始返回 401；2021-11 起退出大陆，基本要走代理；条款禁止自动化抓取 |
| Nasdaq（非官方） | `api.nasdaq.com/api/quote/AAPL/info?assetclass=stocks` | 至少延迟 15 分钟 | 需要浏览器 UA；2026-03 疑似新增人机验证 |

### 聚合库与券商

- **Tushare**：需要 token 和积分，免费档只有非复权日线（50 次/分）；实时报价、分钟 K、盘口都另外付费；美股只有日线；授权为个人非商用；2025-08 曾全站下线约一周。
- **AkShare（Python）**：封装东方财富、新浪、腾讯，A 股走东财接近秒级，美股约晚 15 分钟；声明仅供学术研究。可以当作各非官方接口参数的参考实现（如 `akshare/stock_feature/stock_hist_em.py`）。
- **富途 OpenAPI**：用户本机要运行 OpenD 网关，走 TCP 和 protobuf 真推送；注册牛牛号并完成问卷即可登录（不强制入金，生效时间未确认）；快照接口每 30 秒 60 次；港股 LV2 自 2023-12 起免费，美股深度盘口处于促销期免费，大陆认证用户有 A 股 LV1；2025-09 起大陆新开户基本关闭；条款禁止转发行情。接入成本高（要装网关）。
- **老虎 OpenAPI**：需要开户并入金；美股未订阅时晚 15 分钟；大陆 IP 的港股赠送 L2；基础档可订阅 20 个标的；2022-12 起停止大陆身份证新开户，2025-09 起变通方式也关闭；数据条款原文未查到。

### 如果还要再加

- 想要不需要账户的 A 股：腾讯（轮询，最稳）。东方财富覆盖面最广，但风险最高。
- 想要官方的实时美股：Alpaca（用户自带 key，数据只来自 IEX）。
- 想要券商级的全市场推送：富途 OpenD（前提是用户愿意在本机运行网关）。
