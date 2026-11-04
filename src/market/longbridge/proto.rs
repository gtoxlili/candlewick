//! The protobuf messages of Longbridge's quote socket that the app uses,
//! following <https://github.com/longbridge/openapi-protobufs> (field numbers
//! must match it; gaps in the numbering are fields left out on purpose).

use std::collections::HashMap;

use prost::Message;

/// Command codes: control commands first, then quote requests and pushes.
pub mod cmd {
    pub const AUTH: u8 = 2;
    pub const QUOTE_PROFILE: u8 = 4;
    pub const SUBSCRIBE: u8 = 6;
    pub const UNSUBSCRIBE: u8 = 7;
    pub const STATIC_INFO: u8 = 10;
    pub const QUOTE: u8 = 11;
    pub const DEPTH: u8 = 14;
    pub const TRADES: u8 = 17;
    pub const CANDLESTICKS: u8 = 19;
    pub const HISTORY_CANDLESTICKS: u8 = 27;
    pub const PUSH_QUOTE: u8 = 101;
    pub const PUSH_DEPTH: u8 = 102;
    pub const PUSH_TRADES: u8 = 104;
}

pub mod sub_type {
    pub const QUOTE: i32 = 1;
    pub const DEPTH: i32 = 2;
    pub const TRADE: i32 = 4;
}

pub mod trade_session {
    pub const INTRADAY: i32 = 0;
    pub const PRE: i32 = 1;
    pub const POST: i32 = 2;
    pub const OVERNIGHT: i32 = 3;
    /// In requests: every session, not only the intraday one.
    pub const ALL: i32 = 100;
}

pub mod period {
    pub const MIN_1: i32 = 1;
    pub const MIN_5: i32 = 5;
    pub const MIN_15: i32 = 15;
    pub const MIN_30: i32 = 30;
    pub const MIN_60: i32 = 60;
    pub const DAY: i32 = 1000;
    pub const WEEK: i32 = 2000;
}

pub mod adjust {
    pub const NONE: i32 = 0;
    pub const FORWARD: i32 = 1;
}

/// A failed response's body.
#[derive(Clone, PartialEq, Message)]
pub struct Error {
    #[prost(uint64, tag = "1")]
    pub code: u64,
    #[prost(string, tag = "2")]
    pub msg: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct AuthRequest {
    #[prost(string, tag = "1")]
    pub token: String,
    #[prost(map = "string, string", tag = "2")]
    pub metadata: HashMap<String, String>,
}

/// The answer to [`AuthRequest`].
#[derive(Clone, PartialEq, Message)]
pub struct Session {
    #[prost(string, tag = "1")]
    pub session_id: String,
    /// Epoch milliseconds.
    #[prost(int64, tag = "2")]
    pub expires: i64,
}

#[derive(Clone, PartialEq, Message)]
pub struct QuoteProfileRequest {
    #[prost(string, tag = "1")]
    pub language: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct QuoteProfileResponse {
    #[prost(int64, tag = "1")]
    pub member_id: i64,
    #[prost(string, tag = "2")]
    pub quote_level: String,
    #[prost(int32, tag = "3")]
    pub subscribe_limit: i32,
    #[prost(message, optional, tag = "6")]
    pub quote_level_detail: Option<QuoteLevelDetail>,
}

#[derive(Clone, PartialEq, Message)]
pub struct QuoteLevelDetail {
    #[prost(map = "string, message", tag = "2")]
    pub by_market_code: HashMap<String, MarketPackages>,
}

#[derive(Clone, PartialEq, Message)]
pub struct MarketPackages {
    #[prost(message, repeated, tag = "1")]
    pub packages: Vec<Package>,
    /// Why the market has no packages, e.g. that none were bought.
    #[prost(string, tag = "4")]
    pub warning_msg: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct Package {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, tag = "4")]
    pub description: String,
    /// Epoch seconds.
    #[prost(int64, tag = "5")]
    pub start: i64,
    #[prost(int64, tag = "6")]
    pub end: i64,
}

#[derive(Clone, PartialEq, Message)]
pub struct SecurityRequest {
    #[prost(string, tag = "1")]
    pub symbol: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct MultiSecurityRequest {
    #[prost(string, repeated, tag = "1")]
    pub symbol: Vec<String>,
}

#[derive(Clone, PartialEq, Message)]
pub struct StaticInfoResponse {
    #[prost(message, repeated, tag = "1")]
    pub secu_static_info: Vec<StaticInfo>,
}

#[derive(Clone, PartialEq, Message)]
pub struct StaticInfo {
    #[prost(string, tag = "1")]
    pub symbol: String,
    #[prost(string, tag = "2")]
    pub name_cn: String,
    #[prost(string, tag = "3")]
    pub name_en: String,
    #[prost(string, tag = "7")]
    pub currency: String,
    #[prost(int32, tag = "8")]
    pub lot_size: i32,
    #[prost(string, tag = "17")]
    pub board: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct QuoteResponse {
    #[prost(message, repeated, tag = "1")]
    pub secu_quote: Vec<SecurityQuote>,
}

/// Prices are decimal strings; timestamps epoch seconds.
#[derive(Clone, PartialEq, Message)]
pub struct SecurityQuote {
    #[prost(string, tag = "1")]
    pub symbol: String,
    #[prost(string, tag = "2")]
    pub last_done: String,
    #[prost(string, tag = "3")]
    pub prev_close: String,
    #[prost(string, tag = "4")]
    pub open: String,
    #[prost(string, tag = "5")]
    pub high: String,
    #[prost(string, tag = "6")]
    pub low: String,
    #[prost(int64, tag = "7")]
    pub timestamp: i64,
    #[prost(int64, tag = "8")]
    pub volume: i64,
    #[prost(string, tag = "9")]
    pub turnover: String,
    #[prost(int32, tag = "10")]
    pub trade_status: i32,
    #[prost(message, optional, tag = "11")]
    pub pre_market_quote: Option<ExtendedQuote>,
    #[prost(message, optional, tag = "12")]
    pub post_market_quote: Option<ExtendedQuote>,
    #[prost(message, optional, tag = "13")]
    pub over_night_quote: Option<ExtendedQuote>,
}

/// A US pre-market, post-market or overnight session.
#[derive(Clone, PartialEq, Message)]
pub struct ExtendedQuote {
    #[prost(string, tag = "1")]
    pub last_done: String,
    #[prost(int64, tag = "2")]
    pub timestamp: i64,
    #[prost(int64, tag = "3")]
    pub volume: i64,
    #[prost(string, tag = "4")]
    pub turnover: String,
    #[prost(string, tag = "5")]
    pub high: String,
    #[prost(string, tag = "6")]
    pub low: String,
    /// The close the session's change is measured against.
    #[prost(string, tag = "7")]
    pub prev_close: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct DepthResponse {
    #[prost(string, tag = "1")]
    pub symbol: String,
    #[prost(message, repeated, tag = "2")]
    pub ask: Vec<Depth>,
    #[prost(message, repeated, tag = "3")]
    pub bid: Vec<Depth>,
}

/// One level; `position` counts from 1 at the best price.
#[derive(Clone, PartialEq, Message)]
pub struct Depth {
    #[prost(int32, tag = "1")]
    pub position: i32,
    #[prost(string, tag = "2")]
    pub price: String,
    #[prost(int64, tag = "3")]
    pub volume: i64,
    #[prost(int64, tag = "4")]
    pub order_num: i64,
}

#[derive(Clone, PartialEq, Message)]
pub struct TradesRequest {
    #[prost(string, tag = "1")]
    pub symbol: String,
    #[prost(int32, tag = "2")]
    pub count: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct TradesResponse {
    #[prost(string, tag = "1")]
    pub symbol: String,
    #[prost(message, repeated, tag = "2")]
    pub trades: Vec<Trade>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Trade {
    #[prost(string, tag = "1")]
    pub price: String,
    #[prost(int64, tag = "2")]
    pub volume: i64,
    #[prost(int64, tag = "3")]
    pub timestamp: i64,
    #[prost(string, tag = "4")]
    pub trade_type: String,
    /// 0 neutral, 1 down (the taker sold), 2 up.
    #[prost(int32, tag = "5")]
    pub direction: i32,
    #[prost(int32, tag = "6")]
    pub trade_session: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct CandlesticksRequest {
    #[prost(string, tag = "1")]
    pub symbol: String,
    #[prost(int32, tag = "2")]
    pub period: i32,
    #[prost(int32, tag = "3")]
    pub count: i32,
    #[prost(int32, tag = "4")]
    pub adjust_type: i32,
    #[prost(int32, tag = "5")]
    pub trade_session: i32,
}

/// The answer to both candlestick requests.
#[derive(Clone, PartialEq, Message)]
pub struct CandlesticksResponse {
    #[prost(string, tag = "1")]
    pub symbol: String,
    #[prost(message, repeated, tag = "2")]
    pub candlesticks: Vec<Candlestick>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Candlestick {
    #[prost(string, tag = "1")]
    pub close: String,
    #[prost(string, tag = "2")]
    pub open: String,
    #[prost(string, tag = "3")]
    pub low: String,
    #[prost(string, tag = "4")]
    pub high: String,
    #[prost(int64, tag = "5")]
    pub volume: i64,
    #[prost(string, tag = "6")]
    pub turnover: String,
    #[prost(int64, tag = "7")]
    pub timestamp: i64,
    #[prost(int32, tag = "8")]
    pub trade_session: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct HistoryCandlesticksRequest {
    #[prost(string, tag = "1")]
    pub symbol: String,
    #[prost(int32, tag = "2")]
    pub period: i32,
    #[prost(int32, tag = "3")]
    pub adjust_type: i32,
    /// 1: by offset from a time.
    #[prost(int32, tag = "4")]
    pub query_type: i32,
    #[prost(message, optional, tag = "5")]
    pub offset_request: Option<OffsetQuery>,
    #[prost(int32, tag = "7")]
    pub trade_session: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct OffsetQuery {
    /// 0: before the time, 1: after it.
    #[prost(int32, tag = "1")]
    pub direction: i32,
    /// `YYYYMMDD` in the exchange's time zone.
    #[prost(string, tag = "2")]
    pub date: String,
    /// `HHMM` in the exchange's time zone.
    #[prost(string, tag = "3")]
    pub minute: String,
    #[prost(int32, tag = "4")]
    pub count: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct SubscribeRequest {
    #[prost(string, repeated, tag = "1")]
    pub symbol: Vec<String>,
    #[prost(int32, repeated, tag = "2")]
    pub sub_type: Vec<i32>,
    /// Send the current state right away rather than waiting for a change.
    #[prost(bool, tag = "3")]
    pub is_first_push: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct UnsubscribeRequest {
    #[prost(string, repeated, tag = "1")]
    pub symbol: Vec<String>,
    #[prost(int32, repeated, tag = "2")]
    pub sub_type: Vec<i32>,
    #[prost(bool, tag = "3")]
    pub unsub_all: bool,
}

/// A quote change. Zero or empty fields are unchanged.
#[derive(Clone, PartialEq, Message)]
pub struct PushQuote {
    #[prost(string, tag = "1")]
    pub symbol: String,
    #[prost(string, tag = "3")]
    pub last_done: String,
    #[prost(string, tag = "4")]
    pub open: String,
    #[prost(string, tag = "5")]
    pub high: String,
    #[prost(string, tag = "6")]
    pub low: String,
    #[prost(int64, tag = "7")]
    pub timestamp: i64,
    #[prost(int64, tag = "8")]
    pub volume: i64,
    #[prost(string, tag = "9")]
    pub turnover: String,
    #[prost(int32, tag = "10")]
    pub trade_status: i32,
    #[prost(int32, tag = "11")]
    pub trade_session: i32,
    /// 1: an end-of-day correction, not a live change.
    #[prost(int32, tag = "14")]
    pub tag: i32,
}

/// Changed levels only, by position.
#[derive(Clone, PartialEq, Message)]
pub struct PushDepth {
    #[prost(string, tag = "1")]
    pub symbol: String,
    #[prost(message, repeated, tag = "3")]
    pub ask: Vec<Depth>,
    #[prost(message, repeated, tag = "4")]
    pub bid: Vec<Depth>,
}

#[derive(Clone, PartialEq, Message)]
pub struct PushTrade {
    #[prost(string, tag = "1")]
    pub symbol: String,
    #[prost(message, repeated, tag = "3")]
    pub trade: Vec<Trade>,
}
