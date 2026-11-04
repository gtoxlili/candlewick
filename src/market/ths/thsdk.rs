//! thsdk's runtime: Tonghuashun's own client protocol, as the thsdk Python
//! package (MIT) carries it in a closed-source library (`hq.dylib`,
//! `hq.dll`; `scripts/fetch-thsdk.mjs` fetches it, the bundle ships it). It
//! signs in with a QR code confirmed in the Tonghuashun app and reads the
//! account's watchlist.
//!
//! It runs in a helper process, this executable started with
//! [`HELPER_ARG`], never in the app's own: the closed code sees nothing of
//! the app's memory (the exchanges' keys), its Go runtime's threads and
//! memory go when the helper does, and a crash takes only the helper down.
//! The two speak JSON lines over the helper's stdin and stdout: a request
//! `{method, params?, timeout_ms?}` and its answer `{ok, data?, error?}`,
//! which are the library's own (`CallAlloc`) passed through.

use std::{
    ffi::{CStr, CString, c_char},
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};

use super::ID;
use crate::market::Error;

/// The argument that makes this executable serve as the helper, followed by
/// the library's path.
pub const HELPER_ARG: &str = "--thsdk-helper";
/// The library calls more than this close together are refused; a refusal
/// says how long to wait.
const MAX_RETRY_WAIT_MS: u64 = 1000;
/// Beyond a call's own time limit, for the helper's start and the reply.
const SLACK: Duration = Duration::from_secs(10);

/// Where the bundle keeps the library: the app's `Frameworks` on macOS (as
/// `tauri dev` does beside the build), beside the executable on Windows.
fn library() -> Result<PathBuf, Error> {
    let exe = std::env::current_exe().map_err(|_| missing())?;
    let dir = exe.parent().ok_or_else(missing)?;
    let path = if cfg!(target_os = "macos") {
        dir.join("../Frameworks/hq.dylib")
    } else {
        dir.join("hq.dll")
    };
    path.is_file().then_some(path).ok_or_else(missing)
}

fn missing() -> Error {
    Error::Message(t!("error.thsdkMissing").to_owned())
}

/// A refusal from the library, with its code (`session_unavailable`,
/// `not_authenticated`…) and words.
#[derive(Debug, Deserialize)]
pub struct Refusal {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Deserialize)]
struct Answer {
    ok: bool,
    #[serde(default)]
    data: Value,
    #[serde(default)]
    error: Option<Refusal>,
}

/// What a call came to, when it didn't succeed.
#[derive(Debug)]
pub enum Failure {
    Refused(Refusal),
    /// The helper couldn't start, answer or be understood.
    Broken(Error),
}

impl Failure {
    pub fn refused(&self, code: &str) -> bool {
        matches!(self, Self::Refused(refusal) if refusal.code == code)
    }
}

impl From<Failure> for Error {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Refused(refusal) => Error::Message(t!(
                "error.sourceSays",
                source = ID.name(),
                message = refusal.message
            )),
            Failure::Broken(e) => e,
        }
    }
}

/// A helper process, for as long as it is held: dropping it ends the helper.
pub struct Runtime {
    _child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
}

impl Runtime {
    pub fn start() -> Result<Self, Error> {
        let library = library()?;
        let exe = std::env::current_exe().map_err(|_| missing())?;
        let mut command = Command::new(exe);
        command
            .arg(HELPER_ARG)
            .arg(library)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = command.spawn().map_err(|e| broken(&e))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(broken(&"no pipes"));
        };
        Ok(Self { _child: child, stdin, lines: BufReader::new(stdout).lines() })
    }

    /// Calls `method` and returns its data, waiting at most `limit`.
    pub async fn call(
        &mut self,
        method: &str,
        params: Option<Value>,
        limit: Duration,
    ) -> Result<Value, Failure> {
        let mut request = json!({ "method": method, "timeout_ms": limit.as_millis() as u64 });
        if let Some(params) = params {
            request["params"] = params;
        }
        let mut line = request.to_string();
        line.push('\n');
        let exchange = async {
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.flush().await?;
            self.lines.next_line().await
        };
        let reply = match timeout(limit + SLACK, exchange).await {
            Ok(Ok(Some(reply))) => reply,
            Ok(Ok(None)) => return Err(Failure::Broken(broken(&"the helper quit"))),
            Ok(Err(e)) => return Err(Failure::Broken(broken(&e))),
            Err(_) => return Err(Failure::Broken(broken(&"no answer"))),
        };
        let answer: Answer =
            serde_json::from_str(&reply).map_err(|e| Failure::Broken(broken(&e)))?;
        match (answer.ok, answer.error) {
            (true, _) => Ok(answer.data),
            (false, Some(refusal)) => Err(Failure::Refused(refusal)),
            (false, None) => Err(Failure::Broken(broken(&"a refusal without a reason"))),
        }
    }
}

fn broken(why: &dyn std::fmt::Display) -> Error {
    log::warn!("thsdk helper: {why}");
    Error::Message(t!("error.sourceStopped", source = ID.name()))
}

/// In the helper: serves requests from stdin until it closes, and returns
/// the exit code. `None` when this process isn't the helper.
pub fn serve_if_asked() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != HELPER_ARG {
        return None;
    }
    let library = PathBuf::from(args.next()?);
    let runtime = match Library::load(&library) {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("thsdk helper: cannot load {}: {e}", library.display());
            return Some(2);
        }
    };
    let mut stdout = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let answer = runtime.call_waiting(&line);
        if writeln!(stdout, "{answer}").and_then(|()| stdout.flush()).is_err() {
            break;
        }
    }
    runtime.call(r#"{"method":"_close"}"#);
    Some(0)
}

type CallAlloc = unsafe extern "C" fn(*const c_char) -> *mut c_char;
type FreeResult = unsafe extern "C" fn(*mut c_char);

/// The loaded library's two entry points.
struct Library {
    call_alloc: CallAlloc,
    free_result: FreeResult,
}

impl Library {
    /// One call, its answer as one line of JSON (the library's, re-encoded
    /// on one line; a refusal of its own when that isn't JSON).
    fn call(&self, request: &str) -> String {
        let Ok(request) = CString::new(request) else {
            return refusal("invalid_request", "the request holds a NUL");
        };
        // SAFETY: the library's documented ABI: a NUL-terminated JSON request
        // in, a NUL-terminated answer out that `FreeResult` releases.
        let answer = unsafe {
            let pointer = (self.call_alloc)(request.as_ptr());
            if pointer.is_null() {
                return refusal("no_answer", "the library returned nothing");
            }
            let answer = CStr::from_ptr(pointer).to_string_lossy().into_owned();
            (self.free_result)(pointer);
            answer
        };
        match serde_json::from_str::<Value>(&answer) {
            Ok(value) => value.to_string(),
            Err(_) => refusal("bad_answer", "the library's answer isn't JSON"),
        }
    }

    /// [`Library::call`], again once after the wait a rate-limit refusal asks for.
    fn call_waiting(&self, request: &str) -> String {
        let answer = self.call(request);
        let wait = serde_json::from_str::<Value>(&answer).ok().and_then(|value| {
            let error = value.get("error")?;
            (error.get("code")?.as_str()? == "rate_limited").then_some(())?;
            error.get("retry_after_ms")?.as_u64().filter(|ms| *ms <= MAX_RETRY_WAIT_MS)
        });
        match wait {
            Some(ms) => {
                std::thread::sleep(Duration::from_millis(ms));
                self.call(request)
            }
            None => answer,
        }
    }

    #[cfg(unix)]
    fn load(path: &Path) -> Result<Self, String> {
        use std::{ffi::c_void, os::unix::ffi::OsStrExt};

        unsafe extern "C" {
            fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
            fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
            fn dlerror() -> *const c_char;
        }
        const RTLD_NOW: i32 = 2;
        let path = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        // SAFETY: dlopen and dlsym with NUL-terminated names; the symbols are
        // the library's exported C functions of these signatures, and the
        // library stays loaded for the life of the process.
        unsafe {
            let handle = dlopen(path.as_ptr(), RTLD_NOW);
            if handle.is_null() {
                return Err(CStr::from_ptr(dlerror()).to_string_lossy().into_owned());
            }
            let call_alloc = dlsym(handle, c"CallAlloc".as_ptr());
            let free_result = dlsym(handle, c"FreeResult".as_ptr());
            if call_alloc.is_null() || free_result.is_null() {
                return Err("CallAlloc or FreeResult is missing".to_owned());
            }
            Ok(Self {
                call_alloc: std::mem::transmute::<*mut c_void, CallAlloc>(call_alloc),
                free_result: std::mem::transmute::<*mut c_void, FreeResult>(free_result),
            })
        }
    }

    #[cfg(windows)]
    fn load(path: &Path) -> Result<Self, String> {
        use windows::{
            Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW},
            core::{HSTRING, s},
        };

        // SAFETY: LoadLibraryW and GetProcAddress with valid strings; the
        // symbols are the library's exported C functions of these
        // signatures (the C and system conventions are one on x64), and the
        // library stays loaded for the life of the process.
        unsafe {
            let module = LoadLibraryW(&HSTRING::from(path)).map_err(|e| e.to_string())?;
            let (Some(call_alloc), Some(free_result)) =
                (GetProcAddress(module, s!("CallAlloc")), GetProcAddress(module, s!("FreeResult")))
            else {
                return Err("CallAlloc or FreeResult is missing".to_owned());
            };
            Ok(Self {
                call_alloc: std::mem::transmute::<unsafe extern "system" fn() -> isize, CallAlloc>(
                    call_alloc,
                ),
                free_result: std::mem::transmute::<unsafe extern "system" fn() -> isize, FreeResult>(
                    free_result,
                ),
            })
        }
    }
}

/// An answer the library didn't give, in its shape.
fn refusal(code: &str, message: &str) -> String {
    json!({ "ok": false, "error": { "code": code, "message": message } }).to_string()
}
