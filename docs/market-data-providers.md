# 行情数据源：接入说明与调研记录

币安和长桥调研于 2026-09-26，Bybit、OKX 和各家的账户接口于 2026-10-01，DEX Screener 和 GeckoTerminal 于 2026-10-11，同花顺于 2026-10-28。结论会随时间变化，接入前请在目标网络（大陆直连或代理）下实测一次。

## 怎么接入一个新数据源

所有取数都在 Rust 端，WebView 只通过 IPC 拿数据。

加密货币交易所同一时间只用一家：设置里的 `Settings.exchange` 选定，自选只保留这家的币对（`Settings::validated` 会移除别家的，换交易所就是这样清掉旧币对的），搜索搜这家、长桥、同花顺和 DEX Screener。

### 加一家加密货币交易所

三家交易所的公开行情形状相同，共用的部分在 `src-tauri/src/market/crypto/`：交易对搜索（`pairs.rs`）、菜单栏报价的 WS 驱动（`quotes.rs`）、行情窗口实时流的驱动（`live.rs`）和本地盘口（`book.rs`）。新交易所只写它不同的地方：

1. `ProviderId` 加一个变体，并放进 `ProviderId::EXCHANGES`；`provider()` 返回实现。
2. 实现 `crypto::Exchange`（`Provider` 随之而来）：
   - 常量：REST 主机和 WS 服务器（按顺序，失败后轮换）、保活帧 `PING`（交易所自己发 ping 的填 `None`）、K 线周期（与时钟对齐、日线从 UTC 0 点开始）。
   - `symbol`：交易所写交易对的方式（`BTCUSDT`、`BTC-USDT`），`validate` 和手动输入都靠它。
   - `pairs` / `history` / `recent_trades`：REST。
   - `quotes_path` / `quotes_subscribe` / `quote`：菜单栏报价连哪、连上后发什么订阅、怎么从一帧里读出最新价和 24 小时前的价格。
   - `live`：行情窗口的 `Session`。它是不碰 I/O 的状态机：`connected` 时清空并订阅，`frame` 处理每一帧，通过 `Out` 发帧、发事件、交成交、或者起一个后台请求（结果回到 `fetched`）。连接、重连退避、保活、成交攒批都由 `live::run` 负责，所以会话逻辑可以直接用帧文本做单测。
3. 实现 `crypto::account::Account`（见下文「持仓」），并在 `market::watch_accounts` 和 `market::check_key` 里加上它。
4. 前端 `src/lib/api.ts` 的 `Exchange` 和 `EXCHANGES` 加上它，设置页的选择器和 API Key 一栏就有了；它的名字写进三种语言的 `src-tauri/locales/*.json` 的 `common.provider`（托盘和页面共用，见 [i18n.md](i18n.md)）。

### 加别的数据源

实现 `Provider` trait：

- `id`：自己的 `ProviderId`。
- `validate`：校验设置页传来的自选项。
- `bare`：只凭代码造一个自选项（AI 接口按 id 读任意品种时用），默认把代码转成大写；代码区分大小写的数据源（链上池子的 Solana 地址）要自己实现。
- `search`：返回候选；`notes` 说明搜不全的原因。候选的 `liquidity` 只有池子有，用来区分同一代币的多个池子。
- `drop_search_cache`：设置窗口关闭时释放搜索缓存。
- `watch`：菜单栏（Windows 上是任务栏）报价的常驻任务，读取 `FeedControl` 里的 symbols、paused 和 credentials，写 `model.quotes`，用 `market::set_status` 报告状态。没有推送的数据源在这里轮询（DEX Screener 每 15 秒一轮，同花顺只在交易时段轮询）。
- `chart_spec` / `history` / `recent_trades` / `stream`：行情窗口的能力说明、K 线翻页、最近成交和实时流。

前端 `src/lib/api.ts` 的 `ProviderId`、`instrumentLabel` 加上对应分支，名字写进 `common.provider`。

`chart_spec` 和账户数据里只放代码和数字，不放给人看的文字：数据源给 `ProviderId`，周期给秒数，统计范围给 `StatsSpan`，合约类型给 `PositionKind`，由显示它的一方按界面语言说出来。错误信息例外，用 `t!` 按当时的语言写。链名和 DEX 名是专有名词，不翻译，所以 `Instrument.venue` 直接存显示用的名字（`Solana`、`Raydium`），搜索时由 Rust 填好。

### 几个容易踩错的约定

- `IntervalSpec.aligned`：K 线是否与时钟对齐（`time % secs == 0`），前端据此自己分桶；不对齐的周期由前端每 30 秒或成交越界时向数据源补拉最新几根。`regular_only`：盘前盘后成交不计入该周期。
- `history`：数据源每页上限比请求的 `limit` 小时就返回能给的（OKX 一页 300 根）；只有返回空才表示没有更早的了，前端据此停止往前翻。
- `ChartSpec.day_offset`：日线开盘时刻相对 UTC 的秒数，用于日期标签。
- `ChartSpec.book_steps`：盘口可选的合并档位，从细到粗；`LiveEvent::Book.books` 按同样顺序各给一份合并好的盘口。留空表示不合并，`books` 只放一份原样的盘口。
- `ChartSpec.book` 为 false 表示这个数据源没有盘口（链上池子），行情窗口侧栏只放成交，AI 接口的 `/v1/market` 也不等盘口；`ChartSpec.onchain` 为 true 表示统计是池子的：`Stats.volume` 为空（只有美元成交额），`Stats.onchain` 带流动性、市值、FDV 和 24 小时买卖笔数。`Stats.high` / `low` / `volume` 都是 `Option`，数据源给不出的填 `None`，页面显示「—」。
- `LiveEvent::Tick`：没有成交记录的价格（轮询到的报价），只推动正在形成的 K 线，不进成交列表。
- `ChartSpec.volume_unit` 是成交数量的单位（池子是代币符号），与 `Stats.volume` 是否有值无关。
- 本地盘口的价格存成 10^-18 为单位的 `u128` 整数（Bybit 的 tick 细到 10^-13，币安 BTC/IDR 价格到 15 亿），同一价格总能落到同一档；转回 `f64` 时与直接解析原字符串的结果一致。
- `Trade.id` 必须随时间递增、对同一笔成交稳定（查询和推送会各来一次，前端按 id 去重），而且小于 2^53，否则 WebView 里的数字会把相邻的 id 并成一个。
- 需要账户的数据源：凭证放在 `credentials.rs` 管理的 `credentials.json`（0600），缺凭证时用 `Status::NoCredentials` 提示，不要重试空转。
- `Quote.open` 是涨跌幅的参考价：币是 24 小时前的价格，股票是最近一次收盘价；`Quote.session` 标注盘前、盘后、夜盘。
- 菜单栏每秒最多重绘一次：OKX 每个币对每秒最多推 10 次，币安是每秒一批。
- 返回 429 的数据源：`http::get_json` 把它映射成 `http::Error::Throttled(Retry-After)`，调用方自己决定退避。GeckoTerminal 的客户端据此关一道闸（见下文）。

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

### 同花顺金融数据 API（`src-tauri/src/market/ths/`）

只有 A 股一个市场：股票（沪、深、北）、指数（交易所的指数，以及同花顺自己的概念、行业板块）、ETF 和 LOF。不需要证券账户，用同花顺账号登录 `fuyao.aicubes.cn` 就能签发 Key，所以没有长桥账户也能看 A 股。用一把真实的 Key 实测过。

和长桥并存：两家的 A 股代码写法相同（`600519.SH`），品种 id 靠来源区分（`longbridge:600519.SH`、`ths:600519.SH`）。搜索两家都搜，同一个代码同时出现时，设置页在这几行后面标出来源；没有「A 股用哪家」的开关。长桥逐笔推送，有分钟 K 和盘口；同花顺只有快照和日 K，但不要账户，能按中文名搜，有同花顺板块和北交所。

接入点与约定：

- `https://fuyao.aicubes.cn`，请求头 `X-api-key`。响应信封 `{code, message, request_id, data}`，`code == 0` 为成功，业务错误也是 HTTP 200。Key 错误或已吊销实测返回 2003（文档写的是 2001）；3001 标的不存在，3002 暂无数据，3004 这类标的不支持该能力，4001 或 HTTP 429 为限流。
- 文档里标着「同花顺AI客户端专用」的能力（高频动向、主力资金等）对外返回 2004。个股的分时、分钟 K、逐笔成交和盘口都没有对外开放。
- 限频：文档说不限累计次数，按负载动态限流；实测连发 25 次没有被拒。
- 连续请求会落到不同的服务器：`data.timestamp` 前后跳几秒，成交量偶尔比上一次少一点。

按品种分接口。品种类型来自搜索结果的 `asset_type`，在进程里缓存（`api::kind`）；自选里只存代码，重启后第一次用到时按代码搜一次。

| 类型 | 报价 | 日 K |
|---|---|---|
| 股票 `a-share` | `/api/a-share/prices/snapshot?thscodes=a,b`，一次多个，混进别的类型会被静默略去 | `/api/a-share/prices/historical`，`adjust=forward`，一次最多跨 10 年，能翻到上市之初 |
| 指数 `a-share-index` | `/api/a-share-index/prices/snapshot?thscodes=`，一次多个，一个指数都没有时返回 3001；`.CSI` 后缀的中证指数报 1002，所以不收 | `/api/a-share-index/prices/historical`，只有近 5 年 |
| ETF `fund-etf` | `/api/fund/market/snapshot?thscode=`，一次一个 | `/api/fund/market/historical`，前复权，只有近 5 年 |
| LOF `fund-lof` | 同 ETF | 没有（3004），行情窗口显示「暂无 K 线数据」 |

- 快照字段：`last_price`、`prev_price`（昨收）、`open_price`、`high_price`、`low_price`、`volume`（股；基金是份，指数是成分股合计的股数）、`turnover`（元）。指数的 `ticker` 是同花顺的内部码（上证指数是 `1A0001`），只用 `thscode`。
- 「近 5 年」是硬边界：起点早于 5 年前的请求整段返回空，哪怕终点在 5 年之内。所以请求的起点钳在边界内（`Kind::reach`），翻页越过边界就不再发请求。
- 同花顺的前复权是减去分红，越早价格越低，茅台 2005 年的前复权开盘价是 -310。价格不为正的日 K 丢掉，翻页翻到那里就停。
- 周 K 由日 K 在本地合成，周一 0 点（北京时间）开盘。接口其实接受文档没写的 `interval=1w`、`1mo`（只限股票），但时间戳落在该周、该月最后一个交易日上，形成中的那根每天换一次时间，所以没用。
- 搜索 `/api/meta/tickers/search?q=&asset_type=a-share,a-share-index,fund-etf,fund-lof&limit=8`：代码、完整代码、中文名的子串都能搜到，拼音不行；名字只有中文。输入 `sh600519` 时先改写成 `600519.SH`。
- 交易日历 `/api/a-share/calendar/trading-days`：一年前到今天的交易日，今天在列表里就是交易日。

轮询（`session.rs`）：

- 交易时段 9:15–11:30、13:00–15:00（各多留 5 分钟等收盘价落定），菜单栏和行情窗口各自每 3 秒读一次快照：沪深交易所的 L1 快照每 3 秒发布一次，读得再快也是同一份。
- 休市时读一次收盘价，然后歇到下一个时段开始，最多睡 10 分钟就看一次钟（机器睡过了开盘，醒来后很快能接上）。周末不发请求；工作日先查一次交易日历，节假日整天不读行情（再隔一小时问一次日历，以防日历当天还没更新）。日历读不到时按工作日处理。
- 行情窗口没有盘口和逐笔成交。两次快照之间成交量的增量记作一笔成交，价格取新快照的最新价，方向按价格涨跌判断（价格不变就沿用上一笔），和 L1 行情软件里的「分笔成交」一样；价格变了而成交量没变（集合竞价）只推动 K 线。成交量比上一次少一半以上视为新的一天；少得不多是落后的服务器给的旧数据，整份跳过。
- `recent_trades` 返回空：同花顺不保存成交，打开行情窗口后成交从第一笔增量开始出现。

条款：文档站没有单独的使用条款页；Key 与同花顺账号绑定，签发数量有上限。应用只作为用户自带 Key 的客户端。

#### 账号自选（`account.rs`、`thsdk.rs`）

在「设置 → 同花顺」扫码登录同花顺账号后，用户在同花顺里的自选和自建分组出现在下拉菜单的「同花顺自选」二级菜单里：先是「我的自选」各行，再是每个分组一个子菜单，都带报价，点一行打开它的行情图。只读，从不写回。报价走上面的数据 API，所以要先填 Key。

同花顺的网页登录走不通：登录页对 curl 返回 401 和一段反爬脚本（chameleon），被 DevTools 控制的 Chrome 被拒（「Nginx forbidden」），普通浏览器能打开页面但登录不进去（2026-10-28）。所以用 thsdk 的运行组件，它走同花顺客户端自己的登录和自选同步协议。

- 运行组件：PyPI 上 thsdk 包（MIT）里的闭源库 `hq.dylib` / `hq.dll`（Go 编译，各约 11 MB，压缩后约 4 MB），接口是 `CallAlloc(json) -> json` 加 `FreeResult`：请求 `{method, params?, timeout_ms?}`，回答 `{ok, data?, error?: {code, message, retry_after_ms?}}`。两次调用至少间隔 50 ms，太快会回 `rate_limited` 和要等的毫秒数。`scripts/fetch-thsdk.mjs` 在 `pnpm install` 之后按哈希下载 thsdk 2.0.2，取出这个平台的库和许可证放到 `src-tauri/thsdk/`；macOS 作为 framework 打进 `Contents/Frameworks`，随应用一起签名公证，Windows 放在 exe 旁边。连 `cargo check` 都要它在（`tauri-build` 检查 framework 和 resources 是否存在）。
- 隔离：库只在辅助进程里加载，也就是这个可执行文件带 `--thsdk-helper <库路径>` 启动，`main` 一开头就转去服务它，不碰 Tauri 和单实例。两边按行交换上面的 JSON；每次同步或扫码起一个，用完关掉 stdin 它就退出。闭源代码因此读不到主进程内存里的交易所 Key，它的 Go 运行时（线程、内存、信号处理）不常驻，崩了也只是辅助进程。同一会话目录同时只开一个。
- 会话：存在配置目录的 `thsdk/account.session`（目录 0700），是运行组件自己的格式（Go gob，里面有它模拟的客户端标识）。`credentials.json` 只记 `thsSignedIn`。约一个月有效；过期后 `auth` 回 `session_unavailable`（读自选回 `not_authenticated`），菜单和设置页提示重新登录，不再重试。退出登录调 `logout` 并删掉文件。
- 扫码：`auth_qrcode {session_dir, timeout_seconds, poll_interval_ms}` 返回 `{state: created, image_url, scan_url}`，之后不带参数每秒问一次（`waiting`…），`authenticated` 即成；放弃时传 `{cancel: true}`。`image_url`（`upass.10jqka.com.cn/scan/creatImg?qrid=…`）是一张 252×252 的 PNG，普通请求就能取到，转成 data URL 显示在设置页；三分钟没扫或关掉设置窗口就作废。
- 读取：先 `auth {session_dir}` 恢复会话，再 `get_account_watchlist`（「我的自选」，`securities: [{code, market}]`）和 `get_account_watchlist_groups`（`{group_order, groups: {id: {name, securities}}}`，`group_order` 里 id 会重复）。市场是同花顺客户端的写法：`USHA`、`USZA` 是沪深 A 股，`USHJ`、`USZJ` 是沪深基金。每 10 分钟读一次，失败后按 1、2、4、8 分钟重试。
- 对应到数据 API 的代码：自选里是同花顺自己的代码（上证指数是 `1A0001`）。代码表 `/api/meta/tickers/list?asset_type=…&limit=10000` 一次给全（约 9,100 条，1.9 MB），它的 `ticker` 就是同花顺的代码，全表不重复，按它对上；市场以 `USH`、`USZ` 开头时先按 `代码.SH`、`代码.SZ` 找，以防写的是交易所代码。只有出现没见过的条目时才下载代码表，用完即放；港股、美股、期货等对不上的只计数。实测一个账号的 7 项自选和 11 个分组 60 多项全部对上。
- 报价：这些代码交给菜单栏的报价轮询（`quotes.rs`），交易时段每 30 秒一轮，打开下拉菜单时立即一轮，休市时只在收盘后读一次。股票和指数每 50 个一批。
- 行情窗口能打开不在自选里的品种：`Model::instrument` 先找自选，再找同步来的同花顺自选和分组。

## 持仓（交易所 API Key）

调研于 2026-10-01，手上没有真实的 Key：签名按文档的示例或公式验证，错误路径用格式正确的假 Key 实测过，读到真实余额的那一步没有实测。

### 结构

- 凭证：`credentials.json` 的 `exchanges`，每家一组 `ApiKey`（OKX 多一个 passphrase）。设置页保存前先调 `Account::check`，只确认 Key 能用，不限制它的权限：应用只读取，从不下单或提币，用不着让用户为此另建 Key。
- 读取：每家实现 `crypto::account::Account` 的三个方法：`check`；`trading`，即交易会改变的余额（现货、交易账户、合约账户）和合约仓位；`savings`，即资金账户和理财。`account::run` 是每家一个的常驻任务：平时两分钟刷新一次，持仓窗口打开时十秒一次，打开下拉菜单时刷新超过 15 秒的数据；`savings` 最多五分钟一次（币安理财接口权重 150）；屏幕关闭时暂停。某一部分被交易所拒绝（Key 没有理财权限、没有合约账户），其余照常显示；没读成（网络、超时、5xx，见 `market::Error::transient`）则整份 `savings` 算失败，沿用上一次读到的，下一轮再读，这样一次抖动不会让理财余额从总资产里消失。第一次就没读成时，账户带上错误说明，表示总额缺了这一块。
- 估值：`portfolio.rs`。价格用 `quotes::snapshot` 取：开一条短连接 WS，订阅持仓币种的 `{币}/USDT` ticker，每个币对拿到第一帧就够，最多等 3 秒。没回应的币对保留上次的价格；连续两次没回应、而且两次都是连接撑到最后、别的币对都答了，才视为未上架，1 小时内不再问（`Kept::take`）。连接断了或一个都没答，是网络的问题，不算数。持仓窗口打开期间改为常驻一条 ticker 订阅（`account::Live`）：余额和仓位仍每 10 秒拉一次，价格逐笔到达后按 1 秒合并重新估值，总资产随行情跳动；窗口关闭即断开。拉全表 REST ticker 太大（币安约 1 MB，OKX 约 360 KB），不适合常驻轮询。没有 USDT 交易对的币用交易所自己给的 USD 估值（Bybit `usdValue`、OKX `eqUsd`），再没有就不计价。
- 口径：合约账户按含未实现盈亏的保证金余额（权益）计入总资产，仓位不再重复计入。24 小时涨跌是「持仓不变的前提下过去 24 小时价格变动带来的盈亏」：币按 `数量 × (现价 − 24h 前价格)`，仓位按 `带符号的美元敞口 × 合约自身的 24h 涨跌比例`，合约行情逐个查询（乘数合约如 `1000PEPEUSDT` 在现货里没有对应币对）。期权不计 24h 涨跌。
- 合并：`Portfolio::of` 把各交易所的估值合成一张图——每个账户的总额与钱包分布（`AccountView`），同一资产跨交易所合并成一行（`held` 列出每个交易所的每个钱包），仓位带交易所标记。持仓窗口、下拉菜单和 AI 接口都读这一个结构。
- 展示：下拉菜单最上面一行总资产及其二级菜单（`bar::Holdings`），持仓窗口（`src/holdings/`，见 [holdings.md](holdings.md)），以及「菜单栏显示总资产」时的系统栏本身（`Settings::holdings_in_bar`）。
- 签名的服务器时间：各家拒绝时间偏差过大的请求时，取一次服务器时间（`crypto::account::Clock`）再重试一次。

### 币安

- 签名：查询串（含 `recvWindow`、`timestamp`）做 HMAC-SHA256，十六进制，作为最后一个参数 `signature`；Key 放 `X-MBX-APIKEY`。2026-01-15 起要求对编码后的字节签名。POST 的 sapi 接口把参数全放查询串、body 留空即可。私有接口只在 `api.binance.com`（及 `api-gcp`、`api1-4`），`data-api.binance.vision` 没有。
- 验证 Key：`GET /sapi/v1/account/apiRestrictions`，要求 `enableReading`（各项权限也在这里，没限制 IP 的 HMAC Key 只能读）。
- 现货 `GET /api/v3/account?omitZeroBalances=true`（权重 20）。其中 `LDBTC` 这类余额是活期理财的凭证，和理财持仓重复，跳过；`LDO` 是真币，2026-10 时币安只有它以 LD 开头。
- 资金 `POST /sapi/v1/asset/get-funding-asset`；理财 `GET /sapi/v1/simple-earn/{flexible,locked}/position`（权重各 150，分页 100 条）。
- U 本位 `fapi.binance.com`：`/fapi/v3/account` 的 `assets[].marginBalance`，仓位 `/fapi/v3/positionRisk`（没有杠杆和全逐仓，另查 `/fapi/v1/symbolConfig`）。币本位 `dapi.binance.com`：`/dapi/v1/account`、`/dapi/v1/positionRisk`；一张 BTC 合约 100 美元，其他 10 美元。
- 只开「读取」的 Key 能读合约账户和仓位（第三方实测，不需要「允许合约」）。合约接口返回 `-2015` 时可能是 Key 比合约账户早建、开了统一账户（Portfolio Margin，要走 `papi`，没有接入），或没有合约账户，只跳过合约部分。
- 错误是 `{code, msg}`：`-2015` Key 无效或无权限，`-2008`/`-2014` Key 不存在或格式错，`-1022` 签名错，`-1021` 时间偏差。

### Bybit

- 签名：`timestamp + apiKey + recvWindow + 查询串` 做 HMAC-SHA256，十六进制；头 `X-BAPI-API-KEY`、`X-BAPI-TIMESTAMP`、`X-BAPI-RECV-WINDOW`、`X-BAPI-SIGN`、`X-BAPI-SIGN-TYPE: 2`。文档的示例没给 secret，单测用的是按公式自算的向量。`api.bytick.com` 在部分地区返回 403。
- 验证 Key：`GET /v5/user/query-api`，任何权限的 Key 都能调（`readOnly`、`permissions` 说明它能做什么）。
- 统一交易账户 `GET /v5/account/wallet-balance?accountType=UNIFIED`，每个币的 `equity` 已含合约未实现盈亏；资金 `GET /v5/asset/transfer/query-account-coins-balance?accountType=FUND`；理财 `GET /v5/earn/position?category=FlexibleSaving|OnChain`，需要单独的 Earn 读取权限。
- 仓位 `GET /v5/position/list`：线性合约必须按 `settleCoin=USDT`、`USDC` 分别查，反向和期权不带参数即可；`limit=200`，按 `nextPageCursor` 翻页。`tradeMode` 已废弃，统一账户的全逐仓是整个账户的设置。
- 错误：`wallet-balance` 和 `position/list` 认证失败时只回 HTTP 401、body 为空，没法知道原因；其他端点是 HTTP 200 加 `retCode`（10002 时间、10003 Key 无效、10004 签名、10005 无权限、10010 IP 不符）。

### OKX

- 签名：`timestamp + "GET" + 路径（含查询串）+ body` 做 HMAC-SHA256，base64；`timestamp` 必须是带毫秒、以 `Z` 结尾的 ISO 8601（`2020-12-08T09:08:57.715Z`），前后 30 秒内有效；头 `OK-ACCESS-KEY`、`OK-ACCESS-SIGN`、`OK-ACCESS-TIMESTAMP`、`OK-ACCESS-PASSPHRASE`。Key 是小写 UUID，不能改大小写。
- **地区**：美国、澳洲用户的 Key 只在 `us.okx.com` 有效，欧洲用户的只在 `eea.okx.com`；在别的域名上返回 `50119`（Key 不存在），和填错 Key 一样。所以遇到 50119 依次换地区，并记住认得 Key 的那个。
- 验证 Key：`GET /api/v5/account/config`（`perm` 说明它能做什么：`read_only`、`trade`、`withdraw`）。
- 交易账户 `GET /api/v5/account/balance` 的 `details[].eq`；`openAvgPx`、`spotUpl` 是现货成本价和浮动盈亏（美元，稳定币和法币为空）。资金 `GET /api/v5/asset/balances`；理财 `GET /api/v5/finance/savings/balance` 和 `/finance/staking-defi/orders-active` 的 `investData`。ETH、SOL 质押（BETH、OKSOL）的余额已经在交易和资金账户里，不能再加。
- 仓位 `GET /api/v5/account/positions`：`pos` 的单位是张（杠杆是币）；`posSide` 为 `net` 时正负号表示方向；`notionalUsd` 是美元名义价值；`upl` 以 `ccy` 计。
- 错误多数是 HTTP 401 加 `{code, msg}`，`code` 是字符串，但路径不存在时是数字，解析要两者都接受：`50102` 时间、`50105` passphrase、`50110` IP、`50111`/`50119` Key、`50113` 签名、`50120`/`50030` 无权限、`50101` 模拟盘 Key。

### DEX Screener + GeckoTerminal（链上池子，`src-tauri/src/market/dexscreener/`）

只在 DEX 池子里交易的代币，和交易所的币对一样加进自选。品种代码是 `<链>/<池子地址>`（`solana/EP2ib6…`，和 DEX Screener 网址一致），`base` 是代币符号（`$WIF`），`quote` 固定 `USD`，`name` 是代币名，`venue` 是链和 DEX 的显示名。价格一律按美元，和 DEX Screener 页面默认一样；池子自己的计价币（SOL、WETH）不出现在界面上。两个服务都不要 key；用户可以另外填一个 CoinGecko 的 key，让 K 线和成交走自己的额度（见下文「CoinGecko key」）。

分工，按各自给得出什么：

| 数据 | 来源 | 接口 |
|---|---|---|
| 搜索 | DEX Screener | `GET /latest/dex/search?q=`，30 条，不按流动性排序（搜 `WIF` 第一条是 robinhood 链上的仿盘），所以本地按「代币符号完全匹配 > 前缀 > 其余」再按流动性排，去掉流动性低于 5,000 美元的，取 6 条。粘贴代币或池子地址也能搜到 |
| 菜单栏报价、行情窗口统计 | DEX Screener | `GET /latest/dex/pairs/{chain}/{addr1,addr2,…}`，一次最多 30 个池子，按链分组请求；返回 `priceUsd`、`priceChange.h24`、`volume.h24`、`liquidity.usd`、`fdv`、`marketCap`、`txns.h24.{buys,sells}`。24 小时前的价格由 `priceUsd / (1 + h24/100)` 倒推；刚建的池子没有 `h24`，涨跌按 0 |
| K 线 | GeckoTerminal | `GET /networks/{network}/pools/{addr}/ohlcv/{minute\|hour\|day}?aggregate=&limit=1000&currency=usd&include_empty_intervals=true&before_timestamp=`，倒序返回，日线从 UTC 0 点开始（实测），`include_empty_intervals` 把没有成交的区间补成平盘 K 线，和交易所一样。周期只有分钟 1/5/15、小时 1/4/12、日 1，行情窗口给 1m 5m 15m 1h 4h 1d |
| 成交 | GeckoTerminal | `GET /networks/{network}/pools/{addr}/trades`，过去 24 小时最后 300 笔，秒级时间戳（`block_timestamp`），`kind` 是对基础代币的 buy/sell：buy 时代币在 `to` 一侧，sell 时在 `from` 一侧，价格和数量取相应一侧。id 用「秒 << 20 \| 成交 id 的 FNV 哈希低 20 位」，和长桥一样稳定且按时间递增 |
| 24 小时最高最低 | GeckoTerminal | 两家都不直接给，用最近 25 根小时 K 线算，每 10 分钟刷一次，中间再并入看到的每个价格 |

实时流（`stream.rs`）全靠轮询：DEX Screener 每 10 秒一次（它的边缘缓存 30 秒更新一次），价格变了就发一个 `Tick` 推动当前 K 线；GeckoTerminal 成交每 20 秒一次，连续没有新成交就翻倍拉长到 60 秒；连着两次 DEX Screener 失败才算离线。所以链上的「实时」是 30 秒左右一跳，不是交易所那种逐笔。没有盘口，侧栏放成交和 24 小时买卖比例条。

限频，实测：

- DEX Screener：文档说这几个端点 300 次/分，没碰到过。响应头 `cache-control: public, max-age=30`（搜索 60），`cf-cache-status: HIT`，也就是价格最快 30 秒一变，轮询再快也只是读缓存。
- GeckoTerminal：文档说 30 次/分，从本机（香港出口，不经代理）实测连发 5 次后就开始 429，之后一分钟里大多是 429，偶尔放行一次；也就是实际上每分钟 5 次左右，而且可能和同一出口的其他用户共用。429 不带 `Retry-After`。所以 `gecko.rs` 收到 429 就关一道闸 30 秒（有 `Retry-After` 按它的，最长 120 秒），闸关着时请求直接本地失败、不上网，前端 5 秒一次的历史重试就不会把配额耗在被拒绝上；成交轮询碰到闸就等闸开再问。打开行情窗口的首批请求是 3 次（当前周期的 K 线、小时 K 线、成交），切周期和往前翻页各加 1 次，紧贴这个预算；预算用光时 K 线晚几十秒出现，报价和统计不受影响（走 DEX Screener）。
- 两家都需要 `Accept: application/json;version=20230302`？只有 GeckoTerminal 要（锁定响应格式版本），DEX Screener 不需要头。

链 id 两家不一样（`ethereum` 对 `eth`，`polygon` 对 `polygon_pos`，`avalanche` 对 `avax`，`sui` 对 `sui-network`……），`chains.rs` 存了 70 来条对照表和显示名，来自 GeckoTerminal `/networks`（2026-10-11 共 226 条，3 页）和 DEX Screener 网址里的链 slug；表里没有的链按同名传给 GeckoTerminal，显示名首字母大写。地址：EVM 的 42 位十六进制统一小写（DEX Screener 返回校验和大小写混合），其他（Solana base58 区分大小写）原样保留。

条款：两家的公开 API 都没有附加条款页，也没要求署名；行情窗口的外链指向 dexscreener.com 的池子页。

#### CoinGecko key（可选，`gecko.rs`）

GeckoTerminal 的数据和 CoinGecko API 的 `/onchain/*` 是同一份：路径一样（`/networks/{network}/pools/{addr}/ohlcv/…`），只是前缀和域名不同。设置 → 链上数据 可以填一个 CoinGecko 的 API key（Demo 免费，付费从 Basic 起），存在 `credentials.json` 的 `coingecko`。有 key 时 K 线、成交和 24 小时区间都走 CoinGecko，不再和同一出口的其他人共用 GeckoTerminal 的限额；DEX Screener 那边不变。调研于 2026-10-11（docs.coingecko.com，价目页）：

- 域名和头：Demo key 用 `https://api.coingecko.com/api/v3` 加 `x-cg-demo-api-key`；付费 key 用 `https://pro-api.coingecko.com/api/v3` 加 `x-cg-pro-api-key`。用错域名会被拒绝：10010 是付费 key 用在 Demo 域名，10011 反之。错误体是 `{"status":{"error_code":…,"error_message":"…"}}`。
- 等级：`GET /key` 返回套餐名、每分钟限额、每月额度和本月剩余，但只对付费档开放。所以探测顺序是：先在 Demo 域名调 `/key`，返回 10005（套餐不支持）就说明这是一把有效的 Demo key；返回 10010 就到付费域名再调一次，拿到套餐；其余错误视为 key 无效。Demo 的额度（每分钟 100 次、每月 10,000 次）来自价目页，接口本身报不出来。
- 配速按探测结果来，`gecko::pace()`：没 key 和 Demo 一样，成交每 20 秒、安静时放慢到 60 秒、区间每 10 分钟（Demo 的数据 60 秒才刷新一次，拉得再快也是读缓存；它的好处是不再被共用的限流拦住，K 线每次都能立刻加载）；付费档数据 10 秒一刷，成交每 10 秒、最慢 30 秒、区间每 5 分钟。每次成功请求扣 1 个月度额度（4xx/5xx 不扣，但计入每分钟限额）。
- 兜底：带 key 的请求碰到 429，关 keyed 闸（`Retry-After`，默认 30 秒，最长 120 秒），期间走公共路线；被拒绝（额度用完、key 失效）关 120 秒并作废探测结果，之后重新探测。付费档本月剩余不足 200 次时轮询改走公共路线，把额度留给翻历史；每 6 小时重读一次 `/key`，中间自己按请求递减。探测失败（网络）60 秒后再试，期间走公共路线，所以 key 坏了也只是退回到没 key 的样子。
- 没有真实的 key 做过实测，上面的错误码和 `/key` 的字段来自文档，解析按文档的例子写了单测。

## 候选数据源（未接入）

结论先说：A 股有了同花顺（见上文：官方接口、免费的 Key，但只有快照和日 K）；美股和港股仍然没有一个数据源能同时做到免费、实时、条款允许、大陆可直连。下面的非官方接口都在网页抓取的灰色地带，放进公开仓库有条款风险，而且随时可能被封。

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

### 链上数据的其他来源

调研于 2026-10-11，为接入 DEX Screener 时比较过：

| 来源 | 能给什么 | 为什么没用 |
|---|---|---|
| DEX Screener 网站的 WebSocket（`io.dexscreener.com`） | 逐笔推送 | 非公开，在 Cloudflare 人机验证后面，随时会断 |
| Birdeye、Moralis、Codex.io、Bitquery | K 线、成交、持币人 | 都要 key，免费档很小 |
| CoinGecko Demo key 的 `/onchain/*` | 和 GeckoTerminal 同一份数据 | 要注册；限频同样 30 次/分，缓存 60 秒 |
| Jupiter、Raydium 自己的接口 | 价格、池子 | 只有 Solana |

### 如果还要再加

- 想要 A 股的分钟 K 和逐笔（同花顺对外不给）：只有非官方接口，腾讯最稳，东方财富覆盖面最广但风险最高。
- 想要官方的实时美股：Alpaca（用户自带 key，数据只来自 IEX）。
- 想要券商级的全市场推送：富途 OpenD（前提是用户愿意在本机运行网关）。
- 想要链上逐笔：没有免费又公开的；GeckoTerminal 付费档（CoinGecko Analyst）成交接口不缓存、可按时间段翻页。
